use super::{EngineError, ForkchoiceState};
use alloy_rpc_types_engine::{
    ExecutionPayloadV1, ForkchoiceUpdated, PayloadStatus, PayloadStatusEnum,
};
use rollup_node_primitives::BlockInfo;
use scroll_alloy_provider::ScrollEngineApi;
use std::sync::Arc;

/// The engine that communicates with the execution layer.
#[derive(Debug, Clone)]
pub struct Engine<EC> {
    /// The engine API client.
    client: Arc<EC>,
    /// The fork choice state of the engine.
    fcs: ForkchoiceState,
}

impl<EC> Engine<EC>
where
    EC: ScrollEngineApi + Sync + 'static,
{
    /// Create a new [`Engine`].
    pub const fn new(client: Arc<EC>, fcs: ForkchoiceState) -> Self {
        Self { client, fcs }
    }

    /// Get a reference to the current fork choice state.
    pub const fn fcs(&self) -> &ForkchoiceState {
        &self.fcs
    }

    /// Update the fork choice state and issue an update to the engine.
    pub async fn update_fcs(
        &mut self,
        head: Option<BlockInfo>,
        safe: Option<BlockInfo>,
        finalized: Option<BlockInfo>,
    ) -> Result<ForkchoiceUpdated, EngineError> {
        tracing::trace!(target: "scroll::engine", ?head, ?safe, ?finalized, current = ?self.fcs, "Updating fork choice state");
        if head.is_none() && safe.is_none() && finalized.is_none() {
            return Err(EngineError::fcs_no_update_provided());
        }

        // clone the fcs before updating it
        let mut fcs = self.fcs.clone();
        fcs.update(head, safe, finalized)?;

        // send the fcs update request to the engine
        let result = self.client.fork_choice_updated_v1(fcs.get_alloy_fcs(), None).await?;

        match &result.payload_status.status {
            PayloadStatusEnum::Invalid { validation_error } => {
                tracing::error!(target: "scroll::engine", ?validation_error, "failed to issue forkchoice");
            }
            PayloadStatusEnum::Syncing => {
                tracing::debug!(target: "scroll::engine", "head has been seen before, but not part of the chain");
            }
            PayloadStatusEnum::Accepted => {
                unreachable!("forkchoice update should never return an `Accepted` status");
            }
            PayloadStatusEnum::Valid => {
                tracing::trace!(target: "scroll::engine", "forkchoice updated");
            }
        };

        // update the internal fcs state if the update was successful
        // If the result is invalid, do not update the fcs
        // If the result is valid or sync, update the fcs
        if !result.is_invalid() {
            self.fcs = fcs;
        }

        Ok(result)
    }

    /// Optimistically sync to the given block.
    pub async fn optimistic_sync(
        &mut self,
        block: BlockInfo,
    ) -> Result<ForkchoiceUpdated, EngineError> {
        tracing::trace!(target: "scroll::engine", ?block, current = ?self.fcs, "Optimistically syncing to block");

        // Update the fork choice state to the new block target
        let mut fcs = self.fcs.clone();
        fcs.update(Some(block), None, None)?;

        // Send the optimistic sync request to the engine
        let result =
            self.client.fork_choice_updated_v1(fcs.get_alloy_optimistic_fcs(), None).await?;

        // update the internal fcs state if the update was successful
        // If the result is invalid, do not update the fcs
        // If the result is valid or sync, update the fcs
        if !result.is_invalid() {
            self.fcs = fcs;
        }

        Ok(result)
    }

    /// Submit a new payload to the engine.
    pub async fn new_payload(
        &self,
        payload: ExecutionPayloadV1,
    ) -> Result<PayloadStatus, EngineError> {
        tracing::trace!(target: "scroll::engine", block_number = payload.block_number, block_hash = ?payload.block_hash, "Submitting new payload to engine");
        let result = self.client.new_payload_v1(payload).await?;

        match &result.status {
            PayloadStatusEnum::Invalid { validation_error } => {
                tracing::error!(target: "scroll::engine", ?validation_error, "execution payload is invalid");
            }
            PayloadStatusEnum::Syncing => {
                tracing::debug!(target: "scroll::engine", "execution client is syncing");
            }
            PayloadStatusEnum::Accepted => {
                tracing::error!(target: "scroll::engine", "execution payload part of side chain");
            }
            PayloadStatusEnum::Valid => {
                tracing::trace!(target: "scroll::engine", "execution payload valid");
            }
        };

        Ok(result)
    }

    /// Build a new payload with the given attributes.
    pub async fn build_payload(
        &self,
        head: Option<BlockInfo>,
        attributes: scroll_alloy_rpc_types_engine::ScrollPayloadAttributes,
    ) -> Result<ForkchoiceUpdated, EngineError> {
        tracing::trace!(target: "scroll::engine", ?attributes, "Building new payload with attributes");

        let mut fcs = self.fcs.clone();
        if let Some(head) = head {
            fcs.update(Some(head), None, None)?;
        }

        let result =
            self.client.fork_choice_updated_v1(fcs.get_alloy_fcs(), Some(attributes)).await?;

        tracing::trace!(target: "scroll::engine", ?result, "Build new payload request completed");

        Ok(result)
    }

    /// Get a payload by its ID.
    pub async fn get_payload(
        &self,
        payload_id: alloy_rpc_types_engine::PayloadId,
    ) -> Result<ExecutionPayloadV1, EngineError> {
        tracing::trace!(target: "scroll::engine", ?payload_id, "Getting payload by ID");
        let payload = self.client.get_payload_v1(payload_id).await?;
        Ok(payload)
    }
}
