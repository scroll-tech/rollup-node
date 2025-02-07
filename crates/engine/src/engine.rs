use crate::{block_info::BlockInfo, payload::matching_payloads};

use super::error::EngineDriverError;
use crate::ExecutionPayloadProvider;
use alloy_rpc_types_engine::{
    ExecutionPayload, ExecutionPayloadV1, ForkchoiceState, ForkchoiceUpdated, PayloadId,
    PayloadStatusEnum,
};
use eyre::Result;
use reth_payload_primitives::PayloadTypes;
use reth_scroll_engine_primitives::ScrollEngineTypes;
use scroll_alloy_provider::ScrollEngineApi;

use tokio::time::Duration;
use tracing::{debug, error, info, trace, warn};

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
    EC: ScrollEngineApi<scroll_alloy_network::Scroll> + Unpin + Send + Sync + 'static,
    P: ExecutionPayloadProvider + Unpin + Send + Sync + 'static,
{
    /// Create a new [`EngineDriver`] from the provided [`ScrollAuthEngineApiProvider`] and
    /// generic execution payload provider.
    pub fn new(client: EC, execution_payload_provider: P) -> Self {
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
                    debug!(target: "engine::driver", ?err, "waiting on engine client");
                    tokio::time::sleep(ENGINE_BACKOFF_INTERVAL).await;
                }
                Ok(status) => {
                    info!(target: "engine::driver", payload_status = ?status.payload_status.status, "engine ready");
                    break;
                }
            }
        }

        Self { client, execution_payload_provider }
    }

    /// Handles an execution payload:
    ///   - Sends the payload to the EL via `engine_newPayloadV1`.
    ///   - Sets the current fork choice for the EL via `engine_forkchoiceUpdatedV1`.
    // #[instrument(skip_all, level = "trace", fields(head = payload_block_hash =
    // %execution_payload.block_hash(), payload_block_num = %execution_payload.block_number()))]
    pub async fn handle_execution_payload(
        &self,
        execution_payload: ExecutionPayload,
        fcs: ForkchoiceState,
    ) -> Result<(), EngineDriverError> {
        // let block_info: BlockInfo = (&execution_payload).into();
        let execution_payload = execution_payload.into_v1();
        self.new_payload(execution_payload).await?;
        // self.set_unsafe_block_info(block_info);
        self.forkchoice_updated(fcs, None).await?;

        Ok(())
    }

    /// Handles a payload attributes:
    ///   - Retrieves the execution payload for block at safe head + 1.
    ///   - If the payload is missing or doesn't match the attributes:
    ///         - Starts payload building task on the EL via `engine_forkchoiceUpdatedV1`, passing
    ///           the provided payload attributes.
    ///         - Retrieve the payload with `engine_getPayloadV1`.
    ///         - Sends the constructed payload to the EL via `engine_newPayloadV1`.
    ///         - Sets the current fork choice for the EL via `engine_forkchoiceUpdatedV1`.
    ///   - If the execution payload matches the attributes:
    ///         - Sets the current fork choice for the EL via `engine_forkchoiceUpdatedV1`,
    ///           advancing the safe head by one.
    // #[instrument(skip_all, level = "trace", fields(head = %self.unsafe_block_info.hash, safe =
    // %self.safe_block_info.hash, finalized = %self.safe_block_info.hash))]
    pub async fn handle_payload_attributes(
        &mut self,
        safe_block_info: BlockInfo,
        fcs: ForkchoiceState,
        mut payload_attributes: <ScrollEngineTypes as PayloadTypes>::PayloadAttributes,
    ) -> Result<(), EngineDriverError> {
        let maybe_execution_payload = self
            .execution_payload_provider
            .execution_payload_by_block((safe_block_info.number + 1).into())
            .await
            .map_err(|_| EngineDriverError::EngineUnavailable)?;
        let payload_attributes_already_inserted_in_chain = maybe_execution_payload
            .as_ref()
            .is_some_and(|ep| matching_payloads(&payload_attributes, ep, safe_block_info.hash));

        if payload_attributes_already_inserted_in_chain {
            // if the payload attributes match the execution payload at block safe + 1,
            // this payload has already been passed to the EN in the form of a P2P gossiped
            // execution payload. We can advance the safe head by one by issuing a
            // forkchoiceUpdated.
            self.forkchoice_updated(fcs, None).await?;
        } else {
            // Otherwise, we construct a block from the payload attributes on top of the current
            // safe head.

            // start payload building with `no_tx_pool = true`.
            payload_attributes.no_tx_pool = true;
            let fc_updated = self.forkchoice_updated(fcs, Some(payload_attributes)).await?;

            // retrieve the execution payload
            let execution_payload = self
                .get_payload(fc_updated.payload_id.expect("payload attributes has been set"))
                .await?;

            // issue the execution payload to the EL and set the new forkchoice
            let _safe_block_inf: BlockInfo = (&execution_payload).into();
            self.new_payload(execution_payload.into_v1()).await?;
            self.forkchoice_updated(fcs, None).await?;
        }

        Ok(())
    }

    /// Calls `engine_newPayloadV1` and logs the result.
    async fn new_payload(
        &self,
        execution_payload: ExecutionPayloadV1,
    ) -> Result<(), EngineDriverError> {
        // TODO: should never enter the `Syncing`, `Accepted` or `Invalid` variants when called from
        // `handle_payload_attributes`.
        match self
            .client
            .new_payload_v1(execution_payload)
            .await
            .map_err(|_| EngineDriverError::EngineUnavailable)?
            .status
        {
            PayloadStatusEnum::Invalid { validation_error } => {
                error!(target: "engine::driver", ?validation_error, "failed to issue new execution payload");
                return Err(EngineDriverError::InvalidExecutionPayload)
            }
            PayloadStatusEnum::Syncing => {
                debug!(target: "engine::driver", "EN syncing");
            }
            PayloadStatusEnum::Accepted => {
                warn!(target: "engine::driver", "execution payload part of side chain");
            }
            PayloadStatusEnum::Valid => {
                trace!(target: "engine::driver", "execution payload valid");
            }
        }

        Ok(())
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
                error!(target: "engine::driver", ?validation_error, "failed to issue forkchoice");
                return Err(EngineDriverError::FcuFailed)
            }
            PayloadStatusEnum::Syncing => {
                debug!(target: "engine::driver", "EN syncing");
            }
            PayloadStatusEnum::Accepted => {
                warn!(target: "engine::driver", "payload attributes part of side chain");
            }
            PayloadStatusEnum::Valid => {
                trace!(target: "engine::driver", "execution payload valid");
            }
        }

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
