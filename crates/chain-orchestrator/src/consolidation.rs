use super::ChainOrchestratorError;
use alloy_provider::Provider;
use futures::{stream::FuturesOrdered, TryStreamExt};
use rollup_node_primitives::{BatchInfo, BlockInfo};
use scroll_alloy_network::Scroll;
use scroll_derivation_pipeline::{BatchDerivationResult, DerivedAttributes};
use scroll_engine::{block_matches_attributes, ForkchoiceState};

/// Reconciles a batch of derived attributes with the L2 chain to produce a reconciliation result.
pub(crate) async fn reconcile_batch<L2P: Provider<Scroll>>(
    l2_provider: L2P,
    batch: BatchDerivationResult,
    fcs: &ForkchoiceState,
) -> Result<BatchReconciliationResult, ChainOrchestratorError> {
    let mut futures = FuturesOrdered::new();
    for attributes in batch.attributes {
        let fut = async {
            // Fetch the block corresponding to the derived attributes from the L2 provider.
            let current_block = l2_provider
                .get_block(attributes.block_number.into())
                .full()
                .await?
                .map(|b| b.into_consensus().map_transactions(|tx| tx.inner.into_inner()));

            let current_block = if let Some(block) = current_block {
                block
            } else {
                // The block does not exist, a reorg is needed.
                return Ok::<_, ChainOrchestratorError>(BlockConsolidationAction::Reorg(attributes));
            };

            // Check if the block matches the derived attributes.
            if block_matches_attributes(
                &attributes.attributes,
                &current_block,
                current_block.parent_hash,
            ) {
                // The block matches the derived attributes and the block is below or equal to the
                // safe current safe head.
                if attributes.block_number <= fcs.safe_block_info().number {
                    Ok::<_, ChainOrchestratorError>(BlockConsolidationAction::Skip(BlockInfo {
                        number: current_block.number,
                        hash: current_block.hash_slow(),
                    }))
                } else {
                    // The block matches the derived attributes, no action is needed.
                    Ok::<_, ChainOrchestratorError>(BlockConsolidationAction::UpdateSafeHead(
                        BlockInfo { number: current_block.number, hash: current_block.hash_slow() },
                    ))
                }
            } else {
                // The block does not match the derived attributes, a reorg is needed.
                Ok::<_, ChainOrchestratorError>(BlockConsolidationAction::Reorg(attributes))
            }
        };
        futures.push_back(fut);
    }

    let actions: Vec<BlockConsolidationAction> = futures.try_collect().await?;
    Ok(BatchReconciliationResult { batch_info: batch.batch_info, actions })
}

/// The result of reconciling a batch with the L2 chain.
#[derive(Debug)]
pub(crate) struct BatchReconciliationResult {
    /// The batch info for the consolidated batch.
    pub batch_info: BatchInfo,
    /// The actions that must be performed on the L2 chain to consolidate the batch.
    pub actions: Vec<BlockConsolidationAction>,
}

/// An action that must be performed on the L2 chain to consolidate a block.
#[derive(Debug, Clone)]
pub(crate) enum BlockConsolidationAction {
    /// Update the safe head to the given block.
    UpdateSafeHead(BlockInfo),
    /// The derived attributes match the L2 chain and the safe head is already at or beyond the
    /// block, so no action is needed.
    Skip(BlockInfo),
    /// Reorganize the chain with the given derived attributes.
    Reorg(DerivedAttributes),
}

impl std::fmt::Display for BlockConsolidationAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UpdateSafeHead(info) => {
                write!(f, "UpdateSafeHead to block {}", info.number)
            }
            Self::Skip(info) => write!(f, "Skip block {}", info.number),
            Self::Reorg(attrs) => {
                write!(f, "Reorg to block {}", attrs.block_number)
            }
        }
    }
}
