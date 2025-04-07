use super::error::EngineDriverError;
use crate::payload::matching_payloads;

use alloy_rpc_types_engine::{
    ExecutionPayload, ExecutionPayloadV1, ForkchoiceState, ForkchoiceUpdated, PayloadId,
    PayloadStatusEnum,
};
use eyre::Result;
use reth_payload_primitives::PayloadTypes;
use reth_scroll_engine_primitives::ScrollEngineTypes;
use reth_scroll_primitives::ScrollBlock;
use rollup_node_primitives::BlockInfo;
use rollup_node_providers::ExecutionPayloadProvider;
use scroll_alloy_provider::ScrollEngineApi;
use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;
use tokio::time::Duration;
use tracing::{debug, error, info, instrument, trace};

const ENGINE_BACKOFF_INTERVAL: Duration = Duration::from_secs(1);

/// The main interface to the Engine API of the EN.
/// Internally maintains the fork state of the chain.
#[derive(Debug)]
pub struct EngineDriver<EC, P> {
    /// The engine API client.
    client: EC,
    /// The execution payload provider
    execution_payload_provider: P,
}

impl<EC, P> EngineDriver<EC, P>
where
    EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
    P: ExecutionPayloadProvider + Unpin + Send + Sync + 'static,
{
    /// Create a new [`EngineDriver`] from the provided [`ScrollEngineApi`] and
    /// [`ExecutionPayloadProvider`].
    pub const fn new(client: EC, execution_payload_provider: P) -> Self {
        Self { client, execution_payload_provider }
    }

    /// Initialize the driver and wait for the Engine server to be ready.
    pub async fn init_and_wait_for_engine(
        client: EC,
        execution_payload_provider: P,
        fcs: ForkchoiceState,
    ) -> Self {
        // wait on engine
        loop {
            match client.fork_choice_updated_v1(fcs, None).await {
                Err(err) => {
                    debug!(target: "scroll::engine::driver", ?err, "waiting on engine client");
                    tokio::time::sleep(ENGINE_BACKOFF_INTERVAL).await;
                }
                Ok(status) => {
                    info!(target: "scroll::engine::driver", payload_status = ?status.payload_status.status, "engine ready");
                    break;
                }
            }
        }

        Self { client, execution_payload_provider }
    }

    /// Handles an execution payload:
    ///   - Sends the payload to the EL via `engine_newPayloadV1`.
    ///   - Sets the current fork choice for the EL via `engine_forkchoiceUpdatedV1`.
    #[instrument(skip_all, level = "trace",
        fields(
            payload_block_hash = %execution_payload.block_hash(),
            payload_block_num = %execution_payload.block_number(),
            fcs = ?fcs
        )
    )]
    pub async fn handle_execution_payload(
        &self,
        execution_payload: ExecutionPayload,
        fcs: ForkchoiceState,
    ) -> Result<(PayloadStatusEnum, PayloadStatusEnum), EngineDriverError> {
        // Convert the payload to the V1 format.
        let execution_payload = execution_payload.into_v1();

        // Issue the new payload to the EN.
        let payload_status = self.new_payload(execution_payload).await?;

        // Invoke the FCU with the new state.
        let fcu = self.forkchoice_updated(fcs, None).await?;

        // We should never have a case where the fork choice is syncing as we have already validated
        // the payload and provided it to the EN.
        debug_assert!(fcu.is_valid());

        Ok((payload_status, fcu.payload_status.status))
    }

    /// Handles a payload attributes:
    ///   - Retrieves the execution payload for block at safe head + 1.
    ///   - If the payload is missing or doesn't match the attributes:
    ///     - Starts payload building task on the EL via `engine_forkchoiceUpdatedV1`, passing the
    ///       provided payload attributes.
    ///     - Retrieve the payload with `engine_getPayloadV1`.
    ///     - Sends the constructed payload to the EL via `engine_newPayloadV1`.
    ///     - Sets the current fork choice for the EL via `engine_forkchoiceUpdatedV1`.
    ///   - If the execution payload matches the attributes:
    ///     - Sets the current fork choice for the EL via `engine_forkchoiceUpdatedV1`, advancing
    ///       the safe head by one.
    #[instrument(skip_all, level = "trace",
        fields(
             safe_block_info = ?safe_block_info,
             fcs = ?fcs,
             payload_attributes = ?payload_attributes
        )
    )]
    pub async fn handle_payload_attributes(
        &mut self,
        safe_block_info: BlockInfo,
        mut fcs: ForkchoiceState,
        mut payload_attributes: <ScrollEngineTypes as PayloadTypes>::PayloadAttributes,
    ) -> Result<(BlockInfo, bool), EngineDriverError> {
        let maybe_execution_payload = self
            .execution_payload_provider
            .execution_payload_by_block((safe_block_info.number + 1).into())
            .await
            .map_err(|_| EngineDriverError::ExecutionPayloadProviderUnavailable)?;
        let payload_attributes_already_inserted_in_chain = maybe_execution_payload
            .as_ref()
            .is_some_and(|ep| matching_payloads(&payload_attributes, ep, safe_block_info.hash));

        if payload_attributes_already_inserted_in_chain {
            // if the payload attributes match the execution payload at block safe + 1,
            // this payload has already been passed to the EN in the form of a P2P gossiped
            // execution payload. We can advance the safe head by one by issuing a
            // forkchoiceUpdated.
            let safe_block_info: BlockInfo =
                maybe_execution_payload.expect("execution payload exists").into();
            fcs.safe_block_hash = safe_block_info.hash;
            self.forkchoice_updated(fcs, None).await?;
            Ok((safe_block_info, false))
        } else {
            // Otherwise, we construct a block from the payload attributes on top of the current
            // safe head.
            fcs.head_block_hash = fcs.safe_block_hash;

            // start payload building with `no_tx_pool = true`.
            payload_attributes.no_tx_pool = true;
            let fc_updated = self.forkchoice_updated(fcs, Some(payload_attributes)).await?;

            // retrieve the execution payload
            let execution_payload = self
                .get_payload(fc_updated.payload_id.expect("payload attributes has been set"))
                .await?;

            // issue the execution payload to the EL
            let safe_block_info: BlockInfo = (&execution_payload).into();
            let result = self.new_payload(execution_payload.into_v1()).await?;

            // we should only have a valid payload when deriving from payload attributes (should not
            // be syncing)!
            debug_assert!(result.is_valid());

            // update the fork choice state with the new block hash.
            fcs.head_block_hash = safe_block_info.hash;
            fcs.safe_block_hash = safe_block_info.hash;
            self.forkchoice_updated(fcs, None).await?;

            Ok((safe_block_info, true))
        }
    }

    /// Builds a new payload from the provided fork choice state and payload attributes.
    pub async fn build_new_payload(
        &self,
        mut fcs: ForkchoiceState,
        payload_attributes: ScrollPayloadAttributes,
    ) -> Result<ScrollBlock, EngineDriverError> {
        // start a payload building job on top of the current unsafe head.
        let fc_updated = self.forkchoice_updated(fcs, Some(payload_attributes)).await?;

        // retrieve the execution payload
        let execution_payload = self
            .get_payload(fc_updated.payload_id.expect("payload attributes has been set"))
            .await?;

        // update the head block hash to the new payload block hash.
        fcs.head_block_hash = execution_payload.block_hash();

        // update the fork choice state with the new block hash.
        self.forkchoice_updated(fcs, None).await?;

        // convert the payload into a block.
        execution_payload.try_into().map_err(|_| EngineDriverError::InvalidExecutionPayload)
    }

    /// Calls `engine_newPayloadV1` and logs the result.
    async fn new_payload(
        &self,
        execution_payload: ExecutionPayloadV1,
    ) -> Result<PayloadStatusEnum, EngineDriverError> {
        // TODO: should never enter the `Syncing`, `Accepted` or `Invalid` variants when called from
        // `handle_payload_attributes`.
        let response = self
            .client
            .new_payload_v1(execution_payload)
            .await
            .map_err(|_| EngineDriverError::EngineUnavailable)?;

        match &response.status {
            PayloadStatusEnum::Invalid { validation_error } => {
                error!(target: "scroll::engine::driver", ?validation_error, "execution payload is invalid");
                return Err(EngineDriverError::InvalidExecutionPayload)
            }
            PayloadStatusEnum::Syncing => {
                debug!(target: "scroll::engine::driver", "execution client is syncing");
            }
            PayloadStatusEnum::Accepted => {
                error!(target: "scroll::engine::driver", "execution payload part of side chain");
            }
            PayloadStatusEnum::Valid => {
                trace!(target: "scroll::engine::driver", "execution payload valid");
            }
        };

        Ok(response.status)
    }

    /// Calls `engine_forkchoiceUpdatedV1` and logs the result.
    async fn forkchoice_updated(
        &self,
        fcs: ForkchoiceState,
        attributes: Option<<ScrollEngineTypes as PayloadTypes>::PayloadAttributes>,
    ) -> Result<ForkchoiceUpdated, EngineDriverError> {
        let forkchoice_updated = self
            .client
            .fork_choice_updated_v1(fcs, attributes)
            .await
            .map_err(|_| EngineDriverError::EngineUnavailable)?;

        // TODO: should never enter the `Syncing`, `Accepted` or `Invalid` variants when called from
        // `handle_payload_attributes`.
        match &forkchoice_updated.payload_status.status {
            PayloadStatusEnum::Invalid { validation_error } => {
                error!(target: "scroll::engine::driver", ?validation_error, "failed to issue forkchoice");
                return Err(EngineDriverError::InvalidFcu)
            }
            PayloadStatusEnum::Syncing => {
                debug!(target: "scroll::engine::driver", "head has been seen before, but not part of the chain");
            }
            PayloadStatusEnum::Accepted => {
                unreachable!("forkchoice update should never return an `Accepted` status");
            }
            PayloadStatusEnum::Valid => {
                trace!(target: "scroll::engine::driver", "forkchoice updated");
            }
        };

        Ok(forkchoice_updated)
    }

    /// Calls `engine_getPayloadV1`.
    async fn get_payload(&self, id: PayloadId) -> Result<ExecutionPayload, EngineDriverError> {
        Ok(self
            .client
            .get_payload_v1(id)
            .await
            .map_err(|_| EngineDriverError::EngineUnavailable)?
            .into())
    }
}
