use super::EngineDriverError;
use alloy_rpc_types_engine::{
    ExecutionPayload, ExecutionPayloadV1, ForkchoiceState as AlloyForkchoiceState,
    ForkchoiceUpdated, PayloadId, PayloadStatusEnum,
};
use eyre::Result;
use reth_payload_primitives::PayloadTypes;
use reth_scroll_engine_primitives::ScrollEngineTypes;
use scroll_alloy_provider::ScrollEngineApi;
use std::sync::Arc;
use tracing::{debug, error, trace};

/// Calls `engine_newPayloadV1` and logs the result.
pub(crate) async fn new_payload<EC>(
    client: Arc<EC>,
    execution_payload: ExecutionPayloadV1,
) -> Result<PayloadStatusEnum, EngineDriverError>
where
    EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
{
    // TODO: should never enter the `Syncing`, `Accepted` or `Invalid` variants when called from
    // `handle_payload_attributes`.
    let response = client
        .new_payload_v1(execution_payload)
        .await
        .map_err(|_| EngineDriverError::EngineUnavailable)?;

    match &response.status {
        PayloadStatusEnum::Invalid { validation_error } => {
            error!(target: "scroll::engine::driver", ?validation_error, "execution payload is invalid");
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
pub(crate) async fn forkchoice_updated<EC>(
    client: Arc<EC>,
    fcs: AlloyForkchoiceState,
    attributes: Option<<ScrollEngineTypes as PayloadTypes>::PayloadAttributes>,
) -> Result<ForkchoiceUpdated, EngineDriverError>
where
    EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
{
    let forkchoice_updated = client
        .fork_choice_updated_v1(fcs, attributes)
        .await
        .map_err(EngineDriverError::ForkchoiceUpdateFailed)?;

    // TODO: should never enter the `Syncing`, `Accepted` or `Invalid` variants when called from
    // `handle_payload_attributes`.
    match &forkchoice_updated.payload_status.status {
        PayloadStatusEnum::Invalid { validation_error } => {
            error!(target: "scroll::engine::driver", ?validation_error, "failed to issue forkchoice");
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
pub(crate) async fn get_payload<EC>(
    client: Arc<EC>,
    id: PayloadId,
) -> Result<ExecutionPayload, EngineDriverError>
where
    EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
{
    Ok(client.get_payload_v1(id).await.map_err(|_| EngineDriverError::EngineUnavailable)?.into())
}
