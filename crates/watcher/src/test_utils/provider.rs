use crate::Block;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use alloy_eips::BlockNumberOrTag;
use alloy_json_rpc::RpcError;
use alloy_network::Ethereum;
use alloy_primitives::{Address, BlockNumber, StorageValue, TxHash, B256, U256, U64};
use alloy_provider::{EthGetBlock, Provider, ProviderCall, RootProvider, RpcWithBlock};
use alloy_rpc_types_eth::{BlockId, Filter, Log, Transaction};
use alloy_transport::TransportResult;

/// A mock implementation of the [`Provider`] trait.
#[derive(Debug)]
pub struct MockProvider {
    blocks: Arc<Mutex<HashMap<BlockNumber, Vec<Block>>>>,
    transactions: HashMap<B256, Transaction>,
    logs: Arc<Mutex<VecDeque<Log>>>,
    finalized_blocks: Arc<Mutex<Vec<Block>>>,
    latest_blocks: Arc<Mutex<Vec<Block>>>,
}

impl MockProvider {
    /// Returns a new [`MockProvider`] from the iterator over blocks, the finalized and the latest
    /// block.
    pub fn new(
        blocks: impl Iterator<Item = Block>,
        transactions: impl Iterator<Item = Transaction>,
        logs: impl Iterator<Item = Log>,
        finalized_blocks: Vec<Block>,
        latest_blocks: Vec<Block>,
    ) -> Self {
        let mut b = HashMap::new();
        for block in blocks {
            b.entry(block.header.number).or_insert(Vec::new()).push(block);
        }
        Self {
            blocks: Arc::new(Mutex::new(b)),
            transactions: transactions.map(|tx| (*tx.inner.tx_hash(), tx)).collect(),
            logs: Arc::new(Mutex::new(logs.collect())),
            finalized_blocks: Arc::new(Mutex::new(finalized_blocks)),
            latest_blocks: Arc::new(Mutex::new(latest_blocks)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for MockProvider {
    fn root(&self) -> &RootProvider<Ethereum> {
        unreachable!("unused calls")
    }

    fn get_chain_id(&self) -> ProviderCall<alloy_rpc_client::NoParams, U64, u64> {
        ProviderCall::Ready(Some(Ok(0)))
    }

    fn get_block(&self, block_id: BlockId) -> EthGetBlock<Block> {
        let val = match block_id {
            BlockId::Hash(_) => unimplemented!("hash query is not supported"),
            BlockId::Number(number_or_tag) => match number_or_tag {
                BlockNumberOrTag::Latest => {
                    let mut blocks = self.latest_blocks.lock().unwrap();
                    if blocks.is_empty() {
                        None
                    } else {
                        blocks.drain(..1).next()
                    }
                }
                BlockNumberOrTag::Finalized => {
                    let mut blocks = self.finalized_blocks.lock().unwrap();
                    if blocks.is_empty() {
                        None
                    } else {
                        blocks.drain(..1).next()
                    }
                }
                BlockNumberOrTag::Number(number) => {
                    let mut blocks = self.blocks.lock().unwrap();
                    blocks.get_mut(&number).and_then(|blocks| {
                        if blocks.len() > 1 {
                            blocks.drain(..1).next()
                        } else {
                            blocks.first().cloned()
                        }
                    })
                }
                _ => unimplemented!("can only query by number, latest or finalized"),
            },
        };
        EthGetBlock::new_provider(
            block_id,
            Box::new(move |_kind| {
                let val = val.clone().ok_or(RpcError::NullResp).map(Some);
                ProviderCall::Ready(Some(val))
            }),
        )
    }

    async fn get_logs(&self, _filter: &Filter) -> TransportResult<Vec<Log>> {
        let logs = self.logs.lock().unwrap().pop_front().map(|l| vec![l]).unwrap_or_default();
        Ok(logs)
    }

    fn get_storage_at(
        &self,
        _address: Address,
        _key: U256,
    ) -> RpcWithBlock<(Address, U256), StorageValue> {
        RpcWithBlock::new_provider(|_| ProviderCall::Ready(Some(Ok(StorageValue::ZERO))))
    }

    fn get_transaction_by_hash(
        &self,
        hash: TxHash,
    ) -> ProviderCall<(TxHash,), Option<Transaction>> {
        ProviderCall::Ready(Some(Ok(self.transactions.get(&hash).cloned())))
    }
}
