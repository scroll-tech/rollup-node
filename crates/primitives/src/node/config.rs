use alloy_chains::NamedChain;
use alloy_primitives::{address, Address};

/// The address of the system contract on Mainnet.
pub const MAINNET_SYSTEM_CONTRAT_ADDRESS: Address =
    address!("8432728A257646449245558B8b7Dbe51A16c7a4D");

/// The address of the system contract on Sepolia.
pub const SEPOLIA_SYSTEM_CONTRAT_ADDRESS: Address =
    address!("C706Ba9fa4fedF4507CB7A898b4766c1bbf9be57");

/// The address of the system contract on devnet.
pub const DEV_SYSTEM_CONTRAT_ADDRESS: Address =
    address!("000000000000000000000000000000000000dead");

/// The L1 start block for Mainnet.
pub const MAINNET_L1_START_BLOCK_NUMBER: u64 = 18318215;

/// The L1 start block for Sepolia.
pub const SEPOLIA_L1_START_BLOCK_NUMBER: u64 = 4041343;

/// The L1 start block for devnet.
pub const DEV_L1_START_BLOCK_NUMBER: u64 = 0;

/// A shared configuration for the node.
#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// The address of the system contract used in consensus.
    pub system_contract_address: Address,
    /// The start block for the L1 sync.
    pub start_l1_block: u64,
}

impl NodeConfig {
    /// Returns the node configuration for mainnet.
    pub const fn mainnet() -> Self {
        Self {
            system_contract_address: MAINNET_SYSTEM_CONTRAT_ADDRESS,
            start_l1_block: MAINNET_L1_START_BLOCK_NUMBER,
        }
    }

    /// Returns the node configuration for sepolia.
    pub const fn sepolia() -> Self {
        Self {
            system_contract_address: SEPOLIA_SYSTEM_CONTRAT_ADDRESS,
            start_l1_block: SEPOLIA_L1_START_BLOCK_NUMBER,
        }
    }

    /// Returns the node configuration for devnet.
    pub const fn dev() -> Self {
        Self {
            system_contract_address: SEPOLIA_SYSTEM_CONTRAT_ADDRESS,
            start_l1_block: SEPOLIA_L1_START_BLOCK_NUMBER,
        }
    }

    /// Returns the node configuration from a [`NamedChain`].
    pub fn from_named_chain(chain: NamedChain) -> Self {
        match chain {
            NamedChain::Scroll => Self::mainnet(),
            NamedChain::ScrollSepolia => Self::sepolia(),
            NamedChain::Dev => Self::dev(),
            _ => panic!("expected Scroll Mainnet, Sepolia or Dev"),
        }
    }
}
