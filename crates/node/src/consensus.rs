use reth_scroll_primitives::ScrollBlock;
use scroll_network::ConsensusError;
use secp256k1::{ecdsa::Signature, PublicKey};

/// A trait for consensus implementations.
pub trait Consensus {
    /// Validates a new block with the given signature.
    fn validate_new_block(
        &self,
        block: &ScrollBlock,
        signature: &Signature,
    ) -> Result<(), ConsensusError>;
}

/// A Proof of Authority consensus instance.
#[derive(Debug)]
pub struct PoAConsensus {
    _authorized_signers: Vec<PublicKey>,
}

impl PoAConsensus {
    /// Creates a new [`PoAConsensus`] consensus instance with the given authorized signers.
    pub const fn new(authorized_signers: Vec<PublicKey>) -> Self {
        Self { _authorized_signers: authorized_signers }
    }
}

impl Consensus for PoAConsensus {
    fn validate_new_block(
        &self,
        _block: &ScrollBlock,
        _signature: &Signature,
    ) -> Result<(), ConsensusError> {
        // TODO: recover the public key from the signature and check if it is in the authorized
        // signers --- CURRENTLY NOOP ---
        Ok(())
    }
}
