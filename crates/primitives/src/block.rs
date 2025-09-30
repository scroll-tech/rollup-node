use alloy_consensus::Header;
use alloy_eips::{BlockNumHash, Decodable2718};
use alloy_primitives::{B256, U256};
use alloy_rpc_types_engine::ExecutionPayload;
use core::{
    cmp::Ordering,
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};
use derive_more::{Deref, DerefMut};
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

#[cfg(feature = "arbitrary")]
impl arbitrary::Arbitrary<'_> for BlockInfo {
    fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
        let number = u.int_in_range(0..=u32::MAX)?;
        let hash = B256::arbitrary(u)?;
        Ok(Self { number: number as u64, hash })
    }
}

/// A wrapper around a type to which a L2 block number is attached.
#[derive(Debug, Deref, DerefMut, Default, Copy, Clone, PartialEq, Eq)]
pub struct WithL2BlockNumber<T> {
    /// The L2 block number.
    pub l2_block: u64,
    /// The wrapped type.
    #[deref]
    #[deref_mut]
    pub inner: T,
}

impl<T> WithL2BlockNumber<T> {
    /// Returns a new instance of a [`WithL2BlockNumber`] wrapper.
    pub const fn new(l2_block: u64, inner: T) -> Self {
        Self { l2_block, inner }
    }
}

impl<T: Future + Unpin> Future for WithL2BlockNumber<T> {
    type Output = WithL2BlockNumber<<T as Future>::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let block_number = self.l2_block;
        let inner = ready!(Pin::new(&mut self.get_mut().inner).poll(cx));
        Poll::Ready(WithL2BlockNumber::new(block_number, inner))
    }
}

/// A wrapper around a type to which a L1 block number is attached.
#[derive(Debug, Deref, DerefMut, Default, Copy, Clone, PartialEq, Eq)]
pub struct WithL1FinalizedBlockNumber<T> {
    /// The block number.
    pub l1_block: u64,
    /// The wrapped type.
    #[deref]
    #[deref_mut]
    pub inner: T,
}

impl<T> WithL1FinalizedBlockNumber<T> {
    /// Returns a new instance of a [`WithL1FinalizedBlockNumber`] wrapper.
    pub const fn new(l1_block: u64, inner: T) -> Self {
        Self { l1_block, inner }
    }
}

/// A wrapper around a type to which a batch information is attached.
#[derive(Debug, Deref, DerefMut, Default, Copy, Clone, PartialEq, Eq)]
pub struct WithBatchInfo<T> {
    /// The index of the batch.
    pub index: u64,
    /// The hash of the batch.
    pub hash: B256,
    /// The wrapped type.
    #[deref]
    #[deref_mut]
    pub inner: T,
}

impl<T> WithBatchInfo<T> {
    /// Returns a new instance of a [`WithBatchInfo`] wrapper.
    pub const fn new(index: u64, hash: B256, inner: T) -> Self {
        Self { index, hash, inner }
    }
}

/// Type alias for a wrapper type with the full L2 metadata.
pub type WithFullL2Meta<T> = WithL1FinalizedBlockNumber<WithL2BlockNumber<WithBatchInfo<T>>>;

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
