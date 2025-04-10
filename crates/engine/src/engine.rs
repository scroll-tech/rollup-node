use crate::{error::EngineDriverError, payload::matching_payloads, ForkchoiceState};
use std::sync::Arc;

use alloy_rpc_types_engine::{
    ExecutionPayload, ExecutionPayloadV1, ForkchoiceUpdated, PayloadId, PayloadStatusEnum,
};
use eyre::Result;
use reth_payload_primitives::PayloadTypes;
use reth_scroll_engine_primitives::ScrollEngineTypes;
use rollup_node_primitives::BlockInfo;
use rollup_node_providers::ExecutionPayloadProvider;
use scroll_alloy_provider::ScrollEngineApi;
use tokio::{sync::Mutex, time::Duration};
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
    /// The current forkchoice state of the engine.
    forkchoice_state: Arc<Mutex<ForkchoiceState>>,
}

impl<EC, P> EngineDriver<EC, P>
where
    EC: ScrollEngineApi<scroll_alloy_network::Scroll> + Unpin + Send + Sync + 'static,
    P: ExecutionPayloadProvider + Unpin + Send + Sync + 'static,
{
    /// Create a new [`EngineDriver`] from the provided [`ScrollEngineApi`] and
    /// [`ExecutionPayloadProvider`].
    pub fn new(
        client: EC,
        execution_payload_provider: P,
        forkchoice_state: ForkchoiceState,
    ) -> Self {
        let forkchoice_state = Arc::new(Mutex::new(forkchoice_state));
        Self { client, execution_payload_provider, forkchoice_state }
    }

    /// Initialize the driver and wait for the Engine server to be ready.
    pub async fn init_and_wait_for_engine(
        client: EC,
        execution_payload_provider: P,
        forkchoice_state: ForkchoiceState,
    ) -> Self {
        // wait on engine
        loop {
            match client.fork_choice_updated_v1(forkchoice_state.get_alloy_fcs(), None).await {
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

        let forkchoice_state = Arc::new(Mutex::new(forkchoice_state));
        Self { client, execution_payload_provider, forkchoice_state }
    }

    /// Handles an execution payload:
    ///   - Sends the payload to the EL via `engine_newPayloadV1`.
    ///   - Sets the current fork choice for the EL via `engine_forkchoiceUpdatedV1`.
    #[instrument(skip_all, level = "trace",
        fields(
            payload_block_hash = %execution_payload.block_hash(),
            payload_block_num = %execution_payload.block_number(),
        )
    )]
    pub async fn handle_execution_payload(
        &self,
        execution_payload: ExecutionPayload,
    ) -> Result<(PayloadStatusEnum, PayloadStatusEnum), EngineDriverError> {
        // Convert the payload to the V1 format.
        let execution_payload = execution_payload.into_v1();

        // Issue the new payload to the EN.
        let payload_status = self.new_payload(execution_payload).await?;

        // Invoke the FCU with the new state.
        let fcu = self.forkchoice_updated(None).await?;

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
    #[instrument(skip_all, level = "trace", fields(payload_attributes = ?payload_attributes.payload_attributes))]
    pub async fn handle_payload_attributes(
        &self,
        mut payload_attributes: <ScrollEngineTypes as PayloadTypes>::PayloadAttributes,
    ) -> Result<(BlockInfo, bool), EngineDriverError> {
        let safe_block_info = self.safe_head().await;
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
            self.update_safe_head(safe_block_info).await;
            self.forkchoice_updated(None).await?;
            Ok((safe_block_info, false))
        } else {
            // Otherwise, we construct a block from the payload attributes on top of the current
            // safe head.
            let safe_block_info = self.safe_head().await;
            self.update_unsafe_head(safe_block_info).await;

            // start payload building with `no_tx_pool = true`.
            payload_attributes.no_tx_pool = true;
            let fc_updated = self.forkchoice_updated(Some(payload_attributes)).await?;

            // retrieve the execution payload.
            let payload_id =
                fc_updated.payload_id.ok_or(EngineDriverError::MissingExecutionPayloadId)?;
            let execution_payload = self.get_payload(payload_id).await?;

            // issue the execution payload to the EL
            let safe_block_info: BlockInfo = (&execution_payload).into();
            let result = self.new_payload(execution_payload.into_v1()).await?;

            // we should only have a valid payload when deriving from payload attributes (should not
            // be syncing)!
            debug_assert!(result.is_valid());

            // update the fork choice state with the new block hash.
            self.update_unsafe_head(safe_block_info).await;
            self.update_safe_head(safe_block_info).await;
            self.forkchoice_updated(None).await?;

            Ok((safe_block_info, true))
        }
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
        attributes: Option<<ScrollEngineTypes as PayloadTypes>::PayloadAttributes>,
    ) -> Result<ForkchoiceUpdated, EngineDriverError> {
        let fcs = self.get_alloy_fcs().await;
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

    /// Returns the [alloy_rpc_types_engine::ForkchoiceState].
    async fn get_alloy_fcs(&self) -> alloy_rpc_types_engine::ForkchoiceState {
        self.forkchoice_state.lock().await.get_alloy_fcs()
    }

    /// Returns the safe head of the [`ForkchoiceState`].
    async fn safe_head(&self) -> BlockInfo {
        *self.forkchoice_state.lock().await.safe_block_info()
    }

    /// Updates the safe head of the [`ForkchoiceState`].
    async fn update_safe_head(&self, block_info: BlockInfo) {
        self.forkchoice_state.lock().await.update_safe_block_info(block_info)
    }

    /// Updates the unsafe head of the [`ForkchoiceState`].
    async fn update_unsafe_head(&self, block_info: BlockInfo) {
        self.forkchoice_state.lock().await.update_unsafe_block_info(block_info)
    }
}
