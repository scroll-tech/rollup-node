use alloy_chains::NamedChain;
use alloy_primitives::{address, Address};

/// The address of the system contract on Sepolia.
pub const SEPOLIA_SYSTEM_CONTRAT_ADDRESS: Address =
    address!("C706Ba9fa4fedF4507CB7A898b4766c1bbf9be57");

/// The address of the system contract on Mainnet.
pub const MAINNET_SYSTEM_CONTRAT_ADDRESS: Address =
    address!("8432728A257646449245558B8b7Dbe51A16c7a4D");

/// A shared configuration for the node.
#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// The address of the system contract used in consensus.
    system_contract_address: Address,
}

impl NodeConfig {
    /// Returns the node configuration for mainnet.
    pub const fn mainnet() -> Self {
        Self { system_contract_address: MAINNET_SYSTEM_CONTRAT_ADDRESS }
    }

    /// Returns the node configuration for sepolia.
    pub const fn sepolia() -> Self {
        Self { system_contract_address: SEPOLIA_SYSTEM_CONTRAT_ADDRESS }
    }

    /// Returns the node configuration from a [`NamedChain`].
    pub fn from_named_chain(chain: NamedChain) -> Self {
        match chain {
            NamedChain::Scroll => Self::mainnet(),
            NamedChain::ScrollSepolia => Self::sepolia(),
            _ => panic!("expected Scroll Mainnet or Sepolia"),
        }
    }

    /// Returns the current system contract address.
    pub const fn system_contract_address(&self) -> Address {
        self.system_contract_address
    }
}
