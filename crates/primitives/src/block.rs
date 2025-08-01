use alloy_eips::{BlockNumHash, Decodable2718};
use alloy_primitives::{B256, U256};
use alloy_rpc_types_engine::ExecutionPayload;
use core::{
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};
use reth_primitives_traits::transaction::signed::SignedTransaction;
use reth_scroll_primitives::{ScrollBlock, ScrollTransactionSigned};
use scroll_alloy_consensus::L1_MESSAGE_TRANSACTION_TYPE;
use std::vec::Vec;

/// The default block difficulty for a scroll block.
pub const DEFAULT_BLOCK_DIFFICULTY: U256 = U256::from_limbs([1, 0, 0, 0]);

/// Information about a block.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct BlockInfo {
    /// The block number.
    pub number: u64,
    /// The block hash.
    pub hash: B256,
}

impl BlockInfo {
    /// Returns a new instance of [`BlockInfo`].
    pub const fn new(number: u64, hash: B256) -> Self {
        Self { number, hash }
    }
}

impl From<ExecutionPayload> for BlockInfo {
    fn from(value: ExecutionPayload) -> Self {
        (&value).into()
    }
}

impl From<&ExecutionPayload> for BlockInfo {
    fn from(value: &ExecutionPayload) -> Self {
        Self { number: value.block_number(), hash: value.block_hash() }
    }
}

impl From<BlockNumHash> for BlockInfo {
    fn from(value: BlockNumHash) -> Self {
        Self { number: value.number, hash: value.hash }
    }
}

impl From<&ScrollBlock> for BlockInfo {
    fn from(value: &ScrollBlock) -> Self {
        Self { number: value.number, hash: value.hash_slow() }
    }
}

#[cfg(feature = "arbitrary")]
impl arbitrary::Arbitrary<'_> for BlockInfo {
    fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
        let number = u.int_in_range(0..=u32::MAX)?;
        let hash = B256::arbitrary(u)?;
        Ok(Self { number: number as u64, hash })
    }
}

/// A wrapper around a type to which a block number is attached.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct WithBlockNumber<T> {
    /// The block number.
    pub number: u64,
    /// The wrapped type.
    pub inner: T,
}

impl<T> WithBlockNumber<T> {
    /// Returns a new instance of a [`WithBlockNumber`] wrapper.
    pub const fn new(number: u64, inner: T) -> Self {
        Self { number, inner }
    }
}

impl<T: Future + Unpin> Future for WithBlockNumber<T> {
    type Output = WithBlockNumber<<T as Future>::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let block_number = self.number;
        let inner = ready!(Pin::new(&mut self.get_mut().inner).poll(cx));
        Poll::Ready(WithBlockNumber::new(block_number, inner))
    }
}

/// This struct represents an L2 block with a vector the hashes of the L1 messages included in the
/// block.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(arbitrary::Arbitrary))]
pub struct L2BlockInfoWithL1Messages {
    /// The block info.
    pub block_info: BlockInfo,
    /// The hashes of the L1 messages included in the block.
    pub l1_messages: Vec<B256>,
}

impl From<&ScrollBlock> for L2BlockInfoWithL1Messages {
    fn from(value: &ScrollBlock) -> Self {
        let block_number = value.number;
        let block_hash = value.hash_slow();
        let l1_messages = value
            .body
            .transactions
            .iter()
            .filter(|tx| tx.is_l1_message())
            .map(|tx| *tx.tx_hash())
            .collect();
        Self { block_info: BlockInfo { number: block_number, hash: block_hash }, l1_messages }
    }
}

impl From<&ExecutionPayload> for L2BlockInfoWithL1Messages {
    fn from(value: &ExecutionPayload) -> Self {
        let block_number = value.block_number();
        let block_hash = value.block_hash();
        let l1_messages = value
            .as_v1()
            .transactions
            .iter()
            .filter_map(|raw| {
                (raw.as_ref().first() == Some(&L1_MESSAGE_TRANSACTION_TYPE))
                    .then(|| {
                        let tx = ScrollTransactionSigned::decode_2718(&mut raw.as_ref()).ok()?;
                        Some(*tx.tx_hash())
                    })
                    .flatten()
            })
            .collect();
        Self { block_info: BlockInfo { number: block_number, hash: block_hash }, l1_messages }
    }
}
