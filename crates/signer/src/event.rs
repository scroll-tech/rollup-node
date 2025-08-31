use alloy_signer::Signature;
use reth_scroll_primitives::ScrollBlock;

/// An enum representing the events that can be emitted by the signer.
#[derive(Debug, Clone)]
pub enum SignerEvent {
    /// A block has been signed by the signer.
    SignedBlock {
        /// The signed block.
        block: ScrollBlock,
        /// The signature of the block.
        signature: Signature,
    },
}
