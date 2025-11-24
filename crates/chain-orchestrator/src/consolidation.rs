use super::ChainOrchestratorError;
use alloy_provider::Provider;
use futures::{stream::FuturesOrdered, TryStreamExt};
use rollup_node_primitives::{
    BatchConsolidationOutcome, BatchInfo, BatchStatus, L2BlockInfoWithL1Messages,
};
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
            if block_matches_attributes(&attributes.attributes, &current_block) {
                // Extract the block info with L1 messages.
                let block_info: L2BlockInfoWithL1Messages = (&current_block).into();

                // The derived attributes match the L2 chain but are associated with a block
                // number less than or equal to the finalized block, so skip.
                if attributes.block_number <= fcs.finalized_block_info().number {
                    Ok::<_, ChainOrchestratorError>(BlockConsolidationAction::Skip(block_info))
                } else {
                    // The block matches the derived attributes but is above the finalized block,
                    // so we need to update the fcs.
                    Ok::<_, ChainOrchestratorError>(BlockConsolidationAction::UpdateFcs(block_info))
                }
            } else {
                // The block does not match the derived attributes, a reorg is needed.
                Ok::<_, ChainOrchestratorError>(BlockConsolidationAction::Reorg(attributes))
            }
        };
        futures.push_back(fut);
    }

    let actions: Vec<BlockConsolidationAction> = futures.try_collect().await?;
    Ok(BatchReconciliationResult {
        batch_info: batch.batch_info,
        actions,
        target_status: batch.target_status,
    })
}

/// The result of reconciling a batch with the L2 chain.
#[derive(Debug)]
pub(crate) struct BatchReconciliationResult {
    /// The batch info for the consolidated batch.
    pub batch_info: BatchInfo,
    /// The actions that must be performed on the L2 chain to consolidate the batch.
    pub actions: Vec<BlockConsolidationAction>,
    /// The target status of the batch after consolidation.
    pub target_status: BatchStatus,
}

impl BatchReconciliationResult {
    /// Aggregates the block consolidation actions into an aggregated set of actions required to
    /// consolidate the L2 chain with the batch.
    pub(crate) fn aggregate_actions(&self) -> AggregatedBatchConsolidationActions {
        let mut actions: Vec<BlockConsolidationAction> = vec![];
        for next in &self.actions {
            if let Some(last) = actions.last_mut() {
                match (last, next) {
                    (last, next) if last.is_update_fcs() && next.is_update_fcs() => {
                        *last = next.clone();
                    }
                    _ => {
                        actions.push(next.clone());
                    }
                }
            } else if !next.is_skip() {
                actions.push(next.clone());
            }
        }
        AggregatedBatchConsolidationActions { actions }
    }

    /// Consumes the reconciliation result and produces the consolidated chain by combining
    /// non-reorg block info with the reorg block results.
    pub(crate) async fn into_batch_consolidation_outcome(
        self,
        reorg_results: Vec<L2BlockInfoWithL1Messages>,
    ) -> Result<BatchConsolidationOutcome, ChainOrchestratorError> {
        let mut consolidate_chain =
            BatchConsolidationOutcome::new(self.batch_info, self.target_status);

        // First append all non-reorg results to the consolidated chain.
        self.actions.into_iter().filter(|action| !action.is_reorg()).for_each(|action| {
            consolidate_chain.push_block(action.into_block_info().expect("must have block info"));
        });

        // Append the reorg results at the end of the consolidated chain.
        for block in reorg_results {
            consolidate_chain.push_block(block);
        }

        Ok(consolidate_chain)
    }
}

/// The aggregated actions that must be performed on the L2 chain to consolidate a batch.
#[derive(Debug, Clone)]
pub(crate) struct AggregatedBatchConsolidationActions {
    /// The aggregated actions that must be performed on the L2 chain to consolidate a batch.
    pub actions: Vec<BlockConsolidationAction>,
}

/// An action that must be performed on the L2 chain to consolidate a block.
#[derive(Debug, Clone)]
pub(crate) enum BlockConsolidationAction {
    /// Update the fcs to the given block.
    UpdateFcs(L2BlockInfoWithL1Messages),
    /// The derived attributes match the L2 chain and the safe head is already at or beyond the
    /// block, so no action is needed.
    Skip(L2BlockInfoWithL1Messages),
    /// Reorganize the chain with the given derived attributes.
    Reorg(DerivedAttributes),
}

impl BlockConsolidationAction {
    /// Returns true if the action is to update the fcs.
    pub(crate) const fn is_update_fcs(&self) -> bool {
        matches!(self, Self::UpdateFcs(_))
    }

    /// Returns true if the action is to skip the block.
    pub(crate) const fn is_skip(&self) -> bool {
        matches!(self, Self::Skip(_))
    }

    /// Returns true if the action is to perform a reorg.
    pub(crate) const fn is_reorg(&self) -> bool {
        matches!(self, Self::Reorg(_))
    }

    /// Consumes the action and returns the block info if the action is to update the safe head or
    /// skip, returns None for reorg.
    pub(crate) fn into_block_info(self) -> Option<L2BlockInfoWithL1Messages> {
        match self {
            Self::UpdateFcs(info) | Self::Skip(info) => Some(info),
            Self::Reorg(_attrs) => None,
        }
    }
}

impl std::fmt::Display for BlockConsolidationAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UpdateFcs(info) => {
                write!(f, "UpdateSafeHead to block {}", info.block_info.number)
            }
            Self::Skip(info) => write!(f, "Skip block {}", info.block_info.number),
            Self::Reorg(attrs) => {
                write!(f, "Reorg to block {}", attrs.block_number)
            }
        }
    }
}
