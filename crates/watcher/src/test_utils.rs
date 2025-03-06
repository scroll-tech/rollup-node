use crate::Block;
use std::collections::HashMap;

use alloy_eips::BlockNumberOrTag;
use alloy_network::Ethereum;
use alloy_primitives::{BlockNumber, TxHash, B256};
use alloy_provider::{Provider, ProviderCall, RootProvider};
use alloy_rpc_types_eth::{BlockId, BlockTransactionsKind, Transaction};
use alloy_transport::{BoxTransport, TransportResult};

/// A mock implementation of the [`Provider`] trait.
#[derive(Debug)]
pub struct MockProvider {
    blocks: HashMap<BlockNumber, Block>,
    transactions: HashMap<B256, Transaction>,
    finalized_block: Block,
    latest_block: Block,
}

impl MockProvider {
    /// Returns a new [`MockProvider`] from the iterator over blocks, the finalized and the latest
    /// block.
    pub fn new(
        blocks: impl Iterator<Item = Block>,
        transactions: impl Iterator<Item = Transaction>,
        finalized_block: Block,
        latest_block: Block,
    ) -> Self {
        Self {
            blocks: blocks.map(|b| (b.header.number, b)).collect(),
            transactions: transactions.map(|tx| (*tx.inner.tx_hash(), tx)).collect(),
            finalized_block,
            latest_block,
        }
    }
}

#[async_trait::async_trait]
impl Provider for MockProvider {
    fn root(&self) -> &RootProvider<BoxTransport, Ethereum> {
        unreachable!("unused calls")
    }

    async fn get_block(
        &self,
        block_id: BlockId,
        _kind: BlockTransactionsKind,
    ) -> TransportResult<Option<Block>> {
        Ok(match block_id {
            BlockId::Hash(_) => unimplemented!("hash query is not supported"),
            BlockId::Number(number_or_tag) => match number_or_tag {
                BlockNumberOrTag::Latest => Some(self.latest_block.clone()),
                BlockNumberOrTag::Finalized => Some(self.finalized_block.clone()),
                BlockNumberOrTag::Number(number) => self.blocks.get(&number).cloned(),
                _ => unimplemented!("can only query by number, latest or finalized"),
            },
        })
    }

    fn get_transaction_by_hash(
        &self,
        hash: TxHash,
    ) -> ProviderCall<BoxTransport, (TxHash,), Option<Transaction>> {
        ProviderCall::Ready(Some(Ok(self.transactions.get(&hash).cloned())))
    }
}
