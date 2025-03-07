use crate::Block;
use alloy_eips::BlockNumberOrTag;
use alloy_json_rpc::RpcError;
use alloy_network::Ethereum;
use alloy_primitives::{BlockNumber, TxHash, B256};
use alloy_provider::{Provider, ProviderCall, RootProvider};
use alloy_rpc_types_eth::{BlockId, BlockTransactionsKind, Filter, Log, Transaction};
use alloy_transport::{BoxTransport, TransportResult};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;

/// A mock implementation of the [`Provider`] trait.
#[derive(Debug)]
pub struct MockProvider {
    blocks: Arc<Mutex<HashMap<BlockNumber, Vec<Block>>>>,
    transactions: HashMap<B256, Transaction>,
    finalized_blocks: Arc<Mutex<Vec<Block>>>,
    latest_blocks: Arc<Mutex<Vec<Block>>>,
}

impl MockProvider {
    /// Returns a new [`MockProvider`] from the iterator over blocks, the finalized and the latest
    /// block.
    pub fn new(
        blocks: impl Iterator<Item = Block>,
        transactions: impl Iterator<Item = Transaction>,
        mut finalized_blocks: Vec<Block>,
        mut latest_blocks: Vec<Block>,
    ) -> Self {
        let mut b = HashMap::new();
        for block in blocks {
            b.entry(block.header.number).or_insert(Vec::new()).push(block);
        }
        finalized_blocks.sort_by(|a, b| a.header.number.cmp(&b.header.number));
        latest_blocks.sort_by(|a, b| a.header.number.cmp(&b.header.number));
        Self {
            blocks: Arc::new(Mutex::new(b)),
            transactions: transactions.map(|tx| (*tx.inner.tx_hash(), tx)).collect(),
            finalized_blocks: Arc::new(Mutex::new(finalized_blocks)),
            latest_blocks: Arc::new(Mutex::new(latest_blocks)),
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
        match block_id {
            BlockId::Hash(_) => unimplemented!("hash query is not supported"),
            BlockId::Number(number_or_tag) => match number_or_tag {
                BlockNumberOrTag::Latest => {
                    let mut blocks = self.latest_blocks.lock().await;
                    let val = if blocks.len() > 1 { blocks.drain(..1).next() } else { None };
                    val.ok_or(RpcError::NullResp).map(Some)
                }
                BlockNumberOrTag::Finalized => {
                    let mut blocks = self.finalized_blocks.lock().await;
                    let val = if blocks.len() > 1 { blocks.drain(..1).next() } else { None };
                    val.ok_or(RpcError::NullResp).map(Some)
                }
                BlockNumberOrTag::Number(number) => {
                    let mut blocks = self.blocks.lock().await;
                    Ok(blocks.get_mut(&number).and_then(|blocks| blocks.drain(..1).next()))
                }
                _ => unimplemented!("can only query by number, latest or finalized"),
            },
        }
    }

    fn get_transaction_by_hash(
        &self,
        hash: TxHash,
    ) -> ProviderCall<BoxTransport, (TxHash,), Option<Transaction>> {
        ProviderCall::Ready(Some(Ok(self.transactions.get(&hash).cloned())))
    }

    async fn get_logs(&self, _filter: &Filter) -> TransportResult<Vec<Log>> {
        Ok(vec![])
    }
}
