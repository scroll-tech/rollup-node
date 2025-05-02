use super::{payload::matching_payloads, EngineDriverError};
use crate::api::*;
use alloy_rpc_types_engine::{
    ExecutionPayloadV1, ForkchoiceState as AlloyForkchoiceState, PayloadStatusEnum,
};
use eyre::Result;
use reth_scroll_primitives::ScrollBlock;
use rollup_node_primitives::{BatchInfo, BlockInfo, ScrollPayloadAttributesWithBatchInfo};
use rollup_node_providers::ExecutionPayloadProvider;
use scroll_alloy_provider::ScrollEngineApi;
use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;
use scroll_network::{BlockImportOutcome, NewBlockWithPeer};
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::time::Duration;
use tracing::instrument;

mod result;
pub(crate) use result::EngineDriverFutureResult;

/// A future that represents a block import job.
type BlockImportFuture = Pin<
    Box<
        dyn Future<
                Output = Result<(Option<BlockInfo>, Option<BlockImportOutcome>), EngineDriverError>,
            > + Send,
    >,
>;

// A boolean type indicating if the L1 consolidation job resulted in a reorg.
type IsReorg = bool;

/// A future that represents an L1 consolidation job.
type L1ConsolidationFuture = Pin<
    Box<dyn Future<Output = Result<(BlockInfo, IsReorg, BatchInfo), EngineDriverError>> + Send>,
>;

/// A future that represents a payload building job.
type PayloadBuildingJobFuture =
    Pin<Box<dyn Future<Output = Result<ScrollBlock, EngineDriverError>> + Send>>;

/// An enum that represents the different types of futures that can be executed by the engine
/// driver. It can be a block import job, an L1 consolidation job, or a payload building job.
pub(crate) enum EngineDriverFuture {
    BlockImport(BlockImportFuture),
    L1Consolidation(L1ConsolidationFuture),
    PayloadBuildingJob(PayloadBuildingJobFuture),
}

impl EngineDriverFuture {
    /// Creates a new [`EngineDriverFuture::BlockImport`] future from the provided parameters.
    pub(crate) fn block_import<EC>(
        client: Arc<EC>,
        block_with_peer: NewBlockWithPeer,
        fcs: AlloyForkchoiceState,
    ) -> Self
    where
        EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
    {
        Self::BlockImport(Box::pin(handle_execution_payload(client, block_with_peer, fcs)))
    }

    /// Creates a new [`EngineDriverFuture::L1Consolidation`] future from the provided parameters.
    pub(crate) fn l1_consolidation<EC, P>(
        client: Arc<EC>,
        execution_payload_provider: P,
        safe_block_info: BlockInfo,
        fcs: AlloyForkchoiceState,
        payload_attributes: ScrollPayloadAttributesWithBatchInfo,
    ) -> Self
    where
        EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
        P: ExecutionPayloadProvider + Unpin + Send + Sync + 'static,
    {
        Self::L1Consolidation(Box::pin(handle_payload_attributes(
            client,
            execution_payload_provider,
            safe_block_info,
            fcs,
            payload_attributes,
        )))
    }

    /// Creates a new [`EngineDriverFuture::PayloadBuildingJob`] future from the provided
    /// parameters.
    pub(crate) fn payload_building_job<EC>(
        client: Arc<EC>,
        fcs: AlloyForkchoiceState,
        block_building_duration: Duration,
        payload_attributes: ScrollPayloadAttributes,
    ) -> Self
    where
        EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
    {
        Self::PayloadBuildingJob(Box::pin(build_new_payload(
            client,
            fcs,
            block_building_duration,
            payload_attributes,
        )))
    }
}

impl Future for EngineDriverFuture {
    type Output = EngineDriverFutureResult;

    /// Polls the [`EngineDriverFuture`] and upon completion, returns the result of the
    /// corresponding future by converting it into an [`EngineDriverFutureResult`].
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<EngineDriverFutureResult> {
        let this = self.get_mut();
        match this {
            Self::BlockImport(fut) => fut.as_mut().poll(cx).map(Into::into),
            Self::L1Consolidation(fut) => fut.as_mut().poll(cx).map(Into::into),
            Self::PayloadBuildingJob(fut) => fut.as_mut().poll(cx).map(Into::into),
        }
    }
}

/// Handles an execution payload:
///   - Sends the payload to the EL via `engine_newPayloadV1`.
///   - Sets the current fork choice for the EL via `engine_forkchoiceUpdatedV1`.
#[instrument(skip_all, level = "trace",
        fields(
            peer_id = %block_with_peer.peer_id,
            block_hash = %block_with_peer.block.hash_slow(),
            fcs = ?fcs
        )
    )]
async fn handle_execution_payload<EC>(
    client: Arc<EC>,
    block_with_peer: NewBlockWithPeer,
    mut fcs: AlloyForkchoiceState,
) -> Result<(Option<BlockInfo>, Option<BlockImportOutcome>), EngineDriverError>
where
    EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
{
    tracing::trace!(target: "scroll::engine::future", ?fcs, ?block_with_peer, "handling execution payload");

    // Unpack the block with peer.
    let NewBlockWithPeer { peer_id, block, signature } = block_with_peer;

    // Extract the block info from the payload.
    let block_info: BlockInfo = (&block).into();

    // Create the execution payload.
    let payload = ExecutionPayloadV1::from_block_slow(&block);

    // Issue the new payload to the EN.
    let payload_status = new_payload(client.clone(), payload).await?;

    // Check if the payload is invalid and return early.
    if let PayloadStatusEnum::Invalid { validation_error } = payload_status {
        tracing::error!(target: "scroll::engine", ?validation_error, "execution payload is invalid");

        // If the payload is invalid, return early.
        return Ok((None, Some(BlockImportOutcome::invalid_block(peer_id))));
    }

    // Update the fork choice state with the new block hash.
    fcs.head_block_hash = block_info.hash;

    // Invoke the FCU with the new state.
    let fcu = forkchoice_updated(client.clone(), fcs, None).await?;

    // TODO: Handle other cases appropriately.
    match (payload_status, fcu.payload_status.status) {
        (PayloadStatusEnum::Valid, PayloadStatusEnum::Valid) => Ok((
            Some(block_info),
            Some(BlockImportOutcome::valid_block(
                peer_id,
                block,
                Into::<Vec<u8>>::into(signature).into(),
            )),
        )),
        _ => Ok((None, None)),
    }
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
///     - Sets the current fork choice for the EL via `engine_forkchoiceUpdatedV1`, advancing the
///       safe head by one.
#[instrument(skip_all, level = "trace",
        fields(
             safe_block_info = ?safe_block_info,
             fcs = ?fcs,
             payload_attributes = ?payload_attributes
        )
    )]
async fn handle_payload_attributes<EC, P>(
    client: Arc<EC>,
    execution_payload_provider: P,
    safe_block_info: BlockInfo,
    mut fcs: AlloyForkchoiceState,
    payload_attributes: ScrollPayloadAttributesWithBatchInfo,
) -> Result<(BlockInfo, IsReorg, BatchInfo), EngineDriverError>
where
    EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
    P: ExecutionPayloadProvider + Unpin + Send + Sync + 'static,
{
    tracing::trace!(target: "scroll::engine::future", ?fcs, ?payload_attributes, "handling payload attributes");

    let ScrollPayloadAttributesWithBatchInfo { mut payload_attributes, batch_info } =
        payload_attributes;

    let maybe_execution_payload = execution_payload_provider
        .execution_payload_for_block((safe_block_info.number + 1).into())
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
        forkchoice_updated(client, fcs, None).await?;
        Ok((safe_block_info, false, batch_info))
    } else {
        // Otherwise, we construct a block from the payload attributes on top of the current
        // safe head.
        fcs.head_block_hash = fcs.safe_block_hash;

        // start payload building with `no_tx_pool = true`.
        payload_attributes.no_tx_pool = true;
        let fc_updated = forkchoice_updated(client.clone(), fcs, Some(payload_attributes)).await?;

        // retrieve the execution payload
        let execution_payload = get_payload(
            client.clone(),
            fc_updated.payload_id.expect("payload attributes has been set"),
        )
        .await?;

        // issue the execution payload to the EL
        let safe_block_info: BlockInfo = (&execution_payload).into();
        let result = new_payload(client.clone(), execution_payload.into_v1()).await?;

        // we should only have a valid payload when deriving from payload attributes (should not
        // be syncing)!
        debug_assert!(result.is_valid());

        // update the fork choice state with the new block hash.
        fcs.head_block_hash = safe_block_info.hash;
        fcs.safe_block_hash = safe_block_info.hash;
        forkchoice_updated(client, fcs, None).await?;

        Ok((safe_block_info, true, batch_info))
    }
}

/// Builds a new payload from the provided fork choice state and payload attributes.
async fn build_new_payload<EC>(
    client: Arc<EC>,
    mut fcs: AlloyForkchoiceState,
    block_building_duration: Duration,
    payload_attributes: ScrollPayloadAttributes,
) -> Result<ScrollBlock, EngineDriverError>
where
    EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
{
    tracing::trace!(target: "scroll::engine::future", ?payload_attributes, "building new payload");

    // start a payload building job on top of the current unsafe head.
    let fc_updated = forkchoice_updated(client.clone(), fcs, Some(payload_attributes)).await?;

    // wait for the payload building to take place.
    tokio::time::sleep(block_building_duration).await;

    // retrieve the execution payload
    let execution_payload = get_payload(
        client.clone(),
        fc_updated.payload_id.expect("payload attributes has been set"),
    )
    .await?;

    // update the head block hash to the new payload block hash.
    fcs.head_block_hash = execution_payload.block_hash();

    // update the fork choice state with the new block hash.
    forkchoice_updated(client, fcs, None).await?;

    // convert the payload into a block.
    execution_payload.try_into().map_err(EngineDriverError::InvalidExecutionPayload)
}
