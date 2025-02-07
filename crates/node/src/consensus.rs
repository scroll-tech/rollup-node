use reth_scroll_primitives::ScrollBlock;
use scroll_network::BlockImportError;
use secp256k1::{ecdsa::Signature, PublicKey};

/// A trait for consensus implementations.
pub trait Consensus {
    /// Validates a new block with the given signature.
    fn validate_new_block(
        &self,
        block: &ScrollBlock,
        signature: &Signature,
    ) -> Result<(), BlockImportError>;
}

/// A PoA consensus instance.
#[derive(Debug)]
pub struct PoAConsensus {
    _authorized_signers: Vec<PublicKey>,
}

impl PoAConsensus {
    /// Creates a new PoA consensus instance with the given authorized signers.
    pub fn new(authorized_signers: Vec<PublicKey>) -> Self {
        Self { _authorized_signers: authorized_signers }
    }
}

impl Consensus for PoAConsensus {
    fn validate_new_block(
        &self,
        _block: &ScrollBlock,
        _signature: &Signature,
    ) -> Result<(), BlockImportError> {
        // TODO: recover the public key from the signature and check if it is in the authorized
        // signers --- CURRENTLY NOOP ---
        return Ok(())
    }
}
