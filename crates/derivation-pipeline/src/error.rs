use rollup_node_providers::L1ProviderError;
use scroll_codec::{CodecError, L2Block};
use scroll_db::DatabaseError;

/// An error occurred during the derivation process.
#[derive(Debug, thiserror::Error)]
pub enum DerivationPipelineError {
    /// Missing L1 messages cursor.
    #[error("missing l1 message queue cursor")]
    MissingL1MessageQueueCursor,
    /// Invalid L1 messages count.
    #[error("invalid l1 messages count: expected {expected}, got {got}")]
    InvalidL1MessagesCount {
        /// The expected count.
        expected: u64,
        /// The actual count.
        got: u64,
    },
    /// Missing L1 message.
    #[error("missing l1 message for L2 block {0:?}")]
    MissingL1Message(L2Block),
    /// Unknown batch.
    #[error("unknown batch for index {0}")]
    UnknownBatch(u64),
    /// An error in the codec.
    #[error(transparent)]
    Codec(#[from] CodecError),
    /// An error in the database.
    #[error(transparent)]
    Database(#[from] DatabaseError),
    /// An error at the L1 provider.
    #[error(transparent)]
    L1Provider(#[from] L1ProviderError),
}
