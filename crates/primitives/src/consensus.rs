use alloy_primitives::Address;

/// An update to the system contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsensusUpdate {
    /// The authorized signer has been updated.
    AuthorizedSigner(Address),
}
