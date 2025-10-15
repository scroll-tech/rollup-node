use alloy_consensus::Header;
use alloy_eips::{BlockNumHash, Decodable2718};
use alloy_primitives::{B256, U256};
use alloy_rpc_types_engine::{ExecutionPayload, ExecutionPayloadV1};
use core::{
    cmp::Ordering,
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
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BlockInfo {
    /// The block number.
    pub number: u64,
    /// The block hash.
    pub hash: B256,
}

impl PartialOrd for BlockInfo {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.number.partial_cmp(&other.number)
    }
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

impl From<&Header> for BlockInfo {
    fn from(value: &Header) -> Self {
        Self { number: value.number, hash: value.hash_slow() }
    }
}

impl From<Header> for BlockInfo {
    fn from(value: Header) -> Self {
        Self { number: value.number, hash: value.hash_slow() }
    }
}

impl std::fmt::Display for BlockInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BlockInfo {{ number: {}, hash: 0x{} }}", self.number, self.hash)
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

/// A type alias for a wrapper around a type to which a L1 finalized block number is attached.
pub type WithFinalizedBlockNumber<T> = WithBlockNumber<T>;

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

/// A type alias for a wrapper around a type to which a finalized batch information is attached.
pub type WithFinalizedBatchInfo<T> = WithBatchInfo<T>;

/// A type alias for a wrapper around a type to which a committed batch information is attached.
pub type WithCommittedBatchInfo<T> = WithBatchInfo<T>;

/// A wrapper around a type to which a batch information is attached.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct WithBatchInfo<T> {
    /// The l1 block number associated with the batch.
    pub number: u64,
    /// The index of the batch.
    pub index: u64,
    /// The wrapped type.
    pub inner: T,
}

impl<T> WithBatchInfo<T> {
    /// Returns a new instance of a [`WithBatchInfo`] wrapper.
    pub const fn new(index: u64, number: u64, inner: T) -> Self {
        Self { index, number, inner }
    }
}

impl<T: Future + Unpin> Future for WithBatchInfo<T> {
    type Output = WithBatchInfo<<T as Future>::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let block_number = self.number;
        let index = self.index;
        let inner = ready!(Pin::new(&mut self.get_mut().inner).poll(cx));
        Poll::Ready(WithBatchInfo::new(index, block_number, inner))
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

impl From<&ExecutionPayloadV1> for L2BlockInfoWithL1Messages {
    fn from(value: &ExecutionPayloadV1) -> Self {
        let block_number = value.block_number;
        let block_hash = value.block_hash;
        let l1_messages = value
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
