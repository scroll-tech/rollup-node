use super::{payload::block_matches_attributes, EngineDriverError};
use crate::{api::*, ForkchoiceState};

use alloy_provider::Provider;
use alloy_rpc_types_engine::{
    ExecutionData, ExecutionPayloadV1, ForkchoiceState as AlloyForkchoiceState, ForkchoiceUpdated,
    PayloadStatusEnum,
};
use eyre::Result;
use reth_scroll_engine_primitives::try_into_block;
use reth_scroll_primitives::ScrollBlock;
use rollup_node_primitives::{
    BatchInfo, BlockInfo, ChainImport, L2BlockInfoWithL1Messages, MeteredFuture,
    ScrollPayloadAttributesWithBatchInfo,
};
use scroll_alloy_hardforks::ScrollHardforks;
use scroll_alloy_network::Scroll;
use scroll_alloy_provider::ScrollEngineApi;
use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;
use scroll_network::BlockImportOutcome;
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
type ChainImportFuture = Pin<
    Box<
        dyn Future<
                Output = Result<
                    (Option<BlockInfo>, Option<BlockImportOutcome>, PayloadStatusEnum),
                    EngineDriverError,
                >,
            > + Send,
    >,
>;

/// A future that represents an L1 consolidation job.
type L1ConsolidationFuture =
    Pin<Box<dyn Future<Output = Result<ConsolidationOutcome, EngineDriverError>> + Send>>;

/// An enum that represents the different outcomes of an L1 consolidation job.
#[derive(Debug, Clone)]
pub enum ConsolidationOutcome {
    /// Represents a successful consolidation outcome with the consolidated block info and batch
    /// info.
    Consolidation(L2BlockInfoWithL1Messages, BatchInfo),
    /// Represents a reorganization outcome with the consolidated block info and batch info.
    Reorg(L2BlockInfoWithL1Messages, BatchInfo),
    /// Represents no action taken during consolidation, typically due to L1 not being synced.
    Skipped(BlockInfo, BatchInfo),
}

impl ConsolidationOutcome {
    /// Returns the consolidated block info.
    pub const fn block_info(&self) -> Option<&L2BlockInfoWithL1Messages> {
        match self {
            Self::Consolidation(info, _) | Self::Reorg(info, _) => Some(info),
            Self::Skipped(_, _) => None,
        }
    }

    /// Returns the batch info associated with the consolidation outcome.
    pub const fn batch_info(&self) -> &BatchInfo {
        match self {
            Self::Consolidation(_, batch_info) |
            Self::Reorg(_, batch_info) |
            Self::Skipped(_, batch_info) => batch_info,
        }
    }

    /// Returns a boolean indicating whether the consolidation outcome is a reorg.
    pub const fn is_reorg(&self) -> bool {
        matches!(self, Self::Reorg(_, _))
    }

    /// Returns a boolean indicating whether the consolidation outcome is a consolidation.
    pub const fn is_consolidate(&self) -> bool {
        matches!(self, Self::Consolidation(_, _))
    }
}

/// A future that represents a new payload processing.
type NewPayloadFuture =
    Pin<Box<dyn Future<Output = Result<ScrollBlock, EngineDriverError>> + Send>>;

/// A future that represents a new payload building job.
pub(crate) type BuildNewPayloadFuture =
    MeteredFuture<Pin<Box<dyn Future<Output = Result<ScrollBlock, EngineDriverError>> + Send>>>;

/// A future that represents a new payload building job.
pub(crate) type OptimisticSyncFuture =
    Pin<Box<dyn Future<Output = Result<ForkchoiceUpdated, EngineDriverError>> + Send>>;

/// An enum that represents the different types of futures that can be executed on the engine API.
/// It can be a block import job, an L1 consolidation job, or a new payload processing.
pub(crate) enum EngineFuture {
    ChainImport(ChainImportFuture),
    L1Consolidation(L1ConsolidationFuture),
    NewPayload(NewPayloadFuture),
    OptimisticSync(OptimisticSyncFuture),
}

impl EngineFuture {
    pub(crate) fn chain_import<EC>(
        client: Arc<EC>,
        chain_import: ChainImport,
        fcs: AlloyForkchoiceState,
    ) -> Self
    where
        EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
    {
        Self::ChainImport(Box::pin(handle_chain_import(client, chain_import, fcs)))
    }

    pub(crate) fn optimistic_sync<EC>(client: Arc<EC>, fcs: AlloyForkchoiceState) -> Self
    where
        EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
    {
        Self::OptimisticSync(Box::pin(forkchoice_updated(client, fcs, None)))
    }

    /// Creates a new [`EngineFuture::L1Consolidation`] future from the provided parameters.
    pub(crate) fn l1_consolidation<EC, P>(
        client: Arc<EC>,
        execution_payload_provider: P,
        fcs: ForkchoiceState,
        payload_attributes: ScrollPayloadAttributesWithBatchInfo,
        l1_synced: bool,
    ) -> Self
    where
        EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
        P: Provider<Scroll> + Unpin + Send + Sync + 'static,
    {
        Self::L1Consolidation(Box::pin(handle_payload_attributes(
            client,
            execution_payload_provider,
            fcs,
            payload_attributes,
            l1_synced,
        )))
    }

    /// Creates a new [`EngineFuture::NewPayload`] future from the provided parameters.
    pub(crate) fn handle_new_payload_job<EC>(
        client: Arc<EC>,
        fcs: AlloyForkchoiceState,
        block: ScrollBlock,
    ) -> Self
    where
        EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
    {
        Self::NewPayload(Box::pin(handle_new_payload(client, fcs, block)))
    }
}

impl Future for EngineFuture {
    type Output = EngineDriverFutureResult;

    /// Polls the [`EngineFuture`] and upon completion, returns the result of the
    /// corresponding future by converting it into an [`EngineDriverFutureResult`].
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<EngineDriverFutureResult> {
        let this = self.get_mut();
        match this {
            Self::ChainImport(fut) => fut.as_mut().poll(cx).map(Into::into),
            Self::L1Consolidation(fut) => fut.as_mut().poll(cx).map(Into::into),
            Self::NewPayload(fut) => fut.as_mut().poll(cx).map(Into::into),
            Self::OptimisticSync(fut) => fut.as_mut().poll(cx).map(Into::into),
        }
    }
}

/// Handles an execution payload:
///   - Sends the payload to the EL via `engine_newPayloadV1`.
///   - Sets the current fork choice for the EL via `engine_forkchoiceUpdatedV1`.
#[instrument(skip_all, level = "trace",
        fields(
            peer_id = %chain_import.peer_id,
            block_hash = %chain_import.chain.last().unwrap().hash_slow(),
            fcs = ?fcs
        )
    )]
async fn handle_chain_import<EC>(
    client: Arc<EC>,
    chain_import: ChainImport,
    mut fcs: AlloyForkchoiceState,
) -> Result<(Option<BlockInfo>, Option<BlockImportOutcome>, PayloadStatusEnum), EngineDriverError>
where
    EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
{
    tracing::trace!(target: "scroll::engine::future", ?fcs, ?chain_import.peer_id, chain = ?chain_import.chain.last().unwrap().hash_slow(), "handling execution payload");

    let ChainImport { chain, peer_id, signature } = chain_import;

    // Extract the block info from the last payload.
    let head = chain.last().unwrap().clone();

    let mut payload_status = None;
    for block in chain {
        // Create the execution payload.
        let payload = ExecutionPayloadV1::from_block_slow(&block);

        // Issue the new payload to the EN.
        let status = new_payload(client.clone(), payload).await?;

        // Check if the payload is invalid and return early.
        if let PayloadStatusEnum::Invalid { ref validation_error } = status {
            tracing::error!(target: "scroll::engine", ?validation_error, "execution payload is invalid");

            // If the payload is invalid, return early.
            return Ok((None, Some(BlockImportOutcome::invalid_block(peer_id)), status));
        }

        payload_status = Some(status);
    }
    let payload_status = payload_status.unwrap();

    // Update the fork choice state with the new block hash.
    let block_info: BlockInfo = (&head).into();
    fcs.head_block_hash = block_info.hash;

    // Invoke the FCU with the new state.
    let fcu = forkchoice_updated(client.clone(), fcs, None).await?;

    // TODO: Handle other cases appropriately.
    match (&payload_status, &fcu.payload_status.status) {
        (PayloadStatusEnum::Valid, PayloadStatusEnum::Valid) => Ok((
            Some(block_info),
            Some(BlockImportOutcome::valid_block(
                peer_id,
                head,
                Into::<Vec<u8>>::into(signature).into(),
            )),
            PayloadStatusEnum::Valid,
        )),
        _ => Ok((None, None, fcu.payload_status.status)),
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
             fcs = ?fcs,
             payload_attributes = ?payload_attributes_with_batch_info
        )
    )]
async fn handle_payload_attributes<EC, P>(
    client: Arc<EC>,
    provider: P,
    fcs: ForkchoiceState,
    payload_attributes_with_batch_info: ScrollPayloadAttributesWithBatchInfo,
    l1_synced: bool,
) -> Result<ConsolidationOutcome, EngineDriverError>
where
    EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
    P: Provider<Scroll> + Unpin + Send + Sync + 'static,
{
    tracing::trace!(target: "scroll::engine::future", ?fcs, ?payload_attributes_with_batch_info, "handling payload attributes");

    let ScrollPayloadAttributesWithBatchInfo { mut payload_attributes, batch_info } =
        payload_attributes_with_batch_info.clone();

    let maybe_execution_payload = provider
        .get_block((fcs.safe_block_info().number + 1).into())
        .full()
        .await
        .map_err(|_| EngineDriverError::ExecutionPayloadProviderUnavailable)?
        .map(|b| b.into_consensus().map_transactions(|tx| tx.inner.into_inner()))
        .filter(|b| block_matches_attributes(&payload_attributes, b, fcs.safe_block_info().hash));

    if let Some(execution_payload) = maybe_execution_payload {
        // if the payload attributes match the execution payload at block safe + 1,
        // this payload has already been passed to the EN in the form of a P2P gossiped
        // execution payload. We can advance the safe head by one by issuing a
        // forkchoiceUpdated.
        let safe_block_info: L2BlockInfoWithL1Messages = (&execution_payload).into();

        // We only need to update the safe block hash if we are advancing the safe head past the
        // finalized head. There is a possible edge case where on startup,
        // when we reconsolidate the latest batch, the finalized head is ahead of the safe
        // head.
        if fcs.safe_block_info().number > fcs.finalized_block_info().number {
            let mut fcs = fcs.get_alloy_fcs();
            fcs.safe_block_hash = safe_block_info.block_info.hash;
            forkchoice_updated(client, fcs, None).await?;
        }
        Ok(ConsolidationOutcome::Consolidation(safe_block_info, batch_info))
    } else if l1_synced {
        let mut fcs = fcs.get_alloy_fcs();
        // Otherwise, we construct a block from the payload attributes on top of the current
        // safe head.
        fcs.head_block_hash = fcs.safe_block_hash;

        // start payload building with `no_tx_pool = true`.
        payload_attributes.no_tx_pool = true;
        let fc_updated = forkchoice_updated(client.clone(), fcs, Some(payload_attributes)).await?;

        // retrieve the execution payload
        let execution_payload = get_payload(
            client.clone(),
            fc_updated.payload_id.ok_or(EngineDriverError::L1ConsolidationMissingPayloadId(
                payload_attributes_with_batch_info,
            ))?,
        )
        .await?;
        // issue the execution payload to the EL
        let safe_block_info: L2BlockInfoWithL1Messages = (&execution_payload).into();
        let result = new_payload(client.clone(), execution_payload.into_v1()).await?;

        // we should only have a valid payload when deriving from payload attributes (should not
        // be syncing)!
        debug_assert!(result.is_valid());

        // update the fork choice state with the new block hash.
        fcs.head_block_hash = safe_block_info.block_info.hash;
        fcs.safe_block_hash = safe_block_info.block_info.hash;
        forkchoice_updated(client, fcs, None).await?;

        Ok(ConsolidationOutcome::Reorg(safe_block_info, batch_info))
    } else {
        tracing::warn!(target: "scroll::engine::future", ?payload_attributes_with_batch_info, "L1 not synced, skipping consolidation of non-matching payload attributes");
        Ok(ConsolidationOutcome::Skipped(*fcs.safe_block_info(), batch_info))
    }
}

/// Builds a new payload from the provided fork choice state and payload attributes.
pub(crate) async fn build_new_payload<EC, CS>(
    client: Arc<EC>,
    chain_spec: Arc<CS>,
    fcs: AlloyForkchoiceState,
    block_building_duration: Duration,
    payload_attributes: ScrollPayloadAttributes,
) -> Result<ScrollBlock, EngineDriverError>
where
    EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
    CS: ScrollHardforks,
{
    tracing::trace!(target: "scroll::engine::future", ?payload_attributes, "building new payload");

    // start a payload building job on top of the current unsafe head.
    let fc_updated =
        forkchoice_updated(client.clone(), fcs, Some(payload_attributes.clone())).await?;

    // wait for the payload building to take place.
    tokio::time::sleep(block_building_duration).await;

    // retrieve the execution payload
    let payload = get_payload(
        client.clone(),
        fc_updated
            .payload_id
            .ok_or(EngineDriverError::PayloadBuildingMissingPayloadId(payload_attributes))?,
    )
    .await?;
    let block = try_into_block(ExecutionData { payload, sidecar: Default::default() }, chain_spec)?;

    Ok(block)
}

/// Handles a new payload by updating the fork choice state and returning the new block.
async fn handle_new_payload<EC>(
    client: Arc<EC>,
    mut fcs: AlloyForkchoiceState,
    block: ScrollBlock,
) -> Result<ScrollBlock, EngineDriverError>
where
    EC: ScrollEngineApi + Unpin + Send + Sync + 'static,
{
    // update the head block hash to the new payload block hash.
    fcs.head_block_hash = block.hash_slow();

    // update the fork choice state with the new block hash.
    forkchoice_updated(client, fcs, None).await?;

    Ok(block)
}
