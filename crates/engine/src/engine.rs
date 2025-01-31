use crate::{block_info::BlockInfo, payload::matching_payloads};

use crate::ExecutionPayloadProvider;
use alloy_rpc_types_engine::{
    ExecutionPayload, ExecutionPayloadV1, ForkchoiceState, PayloadId, PayloadStatusEnum,
};
use eyre::{bail, eyre, Result};
use reth_engine_primitives::PayloadTypes;
use reth_rpc_api::EngineApiClient;
use reth_scroll_engine_primitives::ScrollEngineTypes;
use tokio::time::Duration;
use tracing::{debug, error, info, instrument, trace, warn};

const ENGINE_BACKOFF_INTERVAL: Duration = Duration::from_secs(1);

/// The main interface to the Engine API of the EN.
/// Internally maintains the fork state of the chain.
#[derive(Debug, Clone)]
pub struct EngineDriver<EC, P> {
    /// The engine API client.
    client: EC,
    /// The execution payload provider
    execution_payload_provider: P,
    /// The unsafe L2 block info.
    unsafe_block_info: BlockInfo,
    /// The safe L2 block info.
    safe_block_info: BlockInfo,
    /// The finalized L2 block info.
    finalized_block_info: BlockInfo,
}

impl<EC, P> EngineDriver<EC, P>
where
    EC: EngineApiClient<ScrollEngineTypes> + Sync,
    P: ExecutionPayloadProvider,
{
    /// Initialize the driver and wait for the Engine server to be ready.
    pub async fn init_and_wait_for_engine(
        client: EC,
        execution_payload_provider: P,
        unsafe_head: BlockInfo,
        safe_head: BlockInfo,
        finalized_head: BlockInfo,
    ) -> Self {
        let fcs = ForkchoiceState {
            head_block_hash: unsafe_head.hash,
            safe_block_hash: safe_head.hash,
            finalized_block_hash: finalized_head.hash,
        };

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

        Self {
            client,
            execution_payload_provider,
            unsafe_block_info: unsafe_head,
            safe_block_info: safe_head,
            finalized_block_info: finalized_head,
        }
    }

    /// Set the finalized L2 block info.
    pub fn set_finalized_block_info(&mut self, finalized_info: BlockInfo) {
        self.finalized_block_info = finalized_info;
    }

    /// Set the safe L2 block info.
    pub fn set_safe_block_info(&mut self, safe_info: BlockInfo) {
        self.safe_block_info = safe_info;
    }

    /// Set the unsafe L2 block info.
    pub fn set_unsafe_block_info(&mut self, unsafe_info: BlockInfo) {
        self.unsafe_block_info = unsafe_info;
    }

    /// Returns a [`ForkchoiceState`] from the current state of the [`EngineDriver`].
    const fn forkchoice_state(&self) -> ForkchoiceState {
        ForkchoiceState {
            head_block_hash: self.unsafe_block_info.hash,
            safe_block_hash: self.safe_block_info.hash,
            finalized_block_hash: self.finalized_block_info.hash,
        }
    }

    /// Handles an execution payload:
    ///   - Sends the payload to the EL via `engine_newPayloadV1`.
    ///   - Sets the current fork choice for the EL via `engine_forkchoiceUpdatedV1`.
    #[instrument(skip_all, level = "trace", fields(head = %self.unsafe_block_info.hash, safe = %self.safe_block_info.hash, finalized = %self.safe_block_info.hash, payload_block_hash = %execution_payload.block_hash(), payload_block_num = %execution_payload.block_number()))]
    pub async fn handle_execution_payload(
        &mut self,
        execution_payload: ExecutionPayload,
    ) -> Result<()> {
        let unsafe_block_info = (&execution_payload).into();
        let execution_payload = execution_payload.into_v1();
        self.new_payload(execution_payload).await?;
        self.set_unsafe_block_info(unsafe_block_info);
        self.forkchoice_updated(None).await?;

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
    #[instrument(skip_all, level = "trace", fields(head = %self.unsafe_block_info.hash, safe = %self.safe_block_info.hash, finalized = %self.safe_block_info.hash))]
    pub async fn handle_payload_attributes(
        &mut self,
        mut payload_attributes: <ScrollEngineTypes as PayloadTypes>::PayloadAttributes,
    ) -> Result<()> {
        let maybe_execution_payload = self
            .execution_payload_provider
            .execution_payload_by_block((self.safe_block_info.number + 1).into())
            .await?;
        let payload_attributes_already_inserted_in_chain =
            maybe_execution_payload.as_ref().is_some_and(|ep| {
                matching_payloads(&payload_attributes, ep, self.safe_block_info.hash)
            });

        if payload_attributes_already_inserted_in_chain {
            // if the payload attributes match the execution payload at block safe + 1,
            // this payload has already been passed to the EN in the form of a P2P gossiped
            // execution payload. We can advance the safe head by one by issuing a
            // forkchoiceUpdated.
            self.set_safe_block_info(maybe_execution_payload.expect("exists").into());
            self.forkchoice_updated(None).await?;
        } else {
            // Otherwise, we construct a block from the payload attributes on top of the current
            // safe head.
            self.set_unsafe_block_info(self.safe_block_info);

            // start payload building with `no_tx_pool = true`.
            payload_attributes.no_tx_pool = true;
            let id = self
                .forkchoice_updated(Some(payload_attributes))
                .await?
                .ok_or_else(|| eyre!("missing payload id"))?;

            // retrieve the execution payload
            let execution_payload = self.get_payload(id).await?;

            // issue the execution payload to the EL and set the new forkchoice
            let safe_block_info = (&execution_payload).into();
            self.new_payload(execution_payload.into_v1()).await?;

            self.set_safe_block_info(safe_block_info);
            self.set_unsafe_block_info(safe_block_info);
            self.forkchoice_updated(None).await?;
        }

        Ok(())
    }

    /// Calls `engine_newPayloadV1` and logs the result.
    async fn new_payload(&self, execution_payload: ExecutionPayloadV1) -> Result<()> {
        // TODO: should never enter the `Syncing`, `Accepted` or `Invalid` variants when called from
        // `handle_payload_attributes`.
        match self.client.new_payload_v1(execution_payload).await?.status {
            PayloadStatusEnum::Invalid { validation_error } => {
                error!(target: "engine::driver", ?validation_error, "failed to issue new execution payload");
                bail!("invalid payload: {validation_error}")
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
        attributes: Option<<ScrollEngineTypes as PayloadTypes>::PayloadAttributes>,
    ) -> Result<Option<PayloadId>> {
        let fc = self.forkchoice_state();
        let forkchoice_updated = self.client.fork_choice_updated_v1(fc, attributes).await?;

        // TODO: should never enter the `Syncing`, `Accepted` or `Invalid` variants when called from
        // `handle_payload_attributes`.
        match &forkchoice_updated.payload_status.status {
            PayloadStatusEnum::Invalid { validation_error } => {
                error!(target: "engine::driver", ?validation_error, "failed to issue forkchoice");
                bail!("invalid fork choice: {validation_error}")
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

        Ok(forkchoice_updated.payload_id)
    }

    /// Calls `engine_getPayloadV1`.
    async fn get_payload(&self, id: PayloadId) -> Result<ExecutionPayload> {
        Ok(self.client.get_payload_v1(id).await?.into())
    }
}
