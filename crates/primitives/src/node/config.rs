use alloy_chains::NamedChain;
use alloy_primitives::{address, Address};

/// The address of the Scroll Rollup contract on Mainnet.
pub const MAINNET_ROLLUP_CONTRACT_ADDRESS: Address =
    address!("0xa13BAF47339d63B743e7Da8741db5456DAc1E556");

/// The address of the Scroll Rollup contract on Sepolia.
pub const SEPOLIA_ROLLUP_CONTRACT_ADDRESS: Address =
    address!("0x2D567EcE699Eabe5afCd141eDB7A4f2D0D6ce8a0");

/// The address of the Scroll Rollup contract on Devnet.
pub const DEVNET_ROLLUP_CONTRACT_ADDRESS: Address =
    address!("000000000000000000000000000000000000dead");

/// The address of the Scroll L1 message queue v1 contract on Mainnet.
pub const MAINNET_L1_MESSAGE_QUEUE_V1_CONTRACT_ADDRESS: Address =
    address!("0x0d7E906BD9cAFa154b048cFa766Cc1E54E39AF9B");

/// The address of the Scroll L1 message queue v1 contract on Sepolia.
pub const SEPOLIA_L1_MESSAGE_QUEUE_V1_CONTRACT_ADDRESS: Address =
    address!("0xF0B2293F5D834eAe920c6974D50957A1732de763");

/// The address of the Scroll L1 message queue v1 contract on Devnet.
pub const DEVNET_L1_MESSAGE_QUEUE_V1_CONTRACT_ADDRESS: Address =
    address!("000000000000000000000000000000000001dead");

/// The address of the Scroll L1 message queue v2 contract on Mainnet.
pub const MAINNET_L1_MESSAGE_QUEUE_V2_CONTRACT_ADDRESS: Address =
    address!("0x56971da63A3C0205184FEF096E9ddFc7A8C2D18a");

/// The address of the Scroll L1 message queue v2 contract on Sepolia.
pub const SEPOLIA_L1_MESSAGE_QUEUE_V2_CONTRACT_ADDRESS: Address =
    address!("0xA0673eC0A48aa924f067F1274EcD281A10c5f19F");

/// The address of the Scroll L1 message queue v2 contract on Devnet.
pub const DEVNET_L1_MESSAGE_QUEUE_V2_CONTRACT_ADDRESS: Address =
    address!("000000000000000000000000000000000002dead");

/// The address of the system contract on Mainnet.
pub const MAINNET_SYSTEM_CONTRAT_ADDRESS: Address =
    address!("8432728A257646449245558B8b7Dbe51A16c7a4D");

/// The address of the system contract on Sepolia.
pub const SEPOLIA_SYSTEM_CONTRAT_ADDRESS: Address =
    address!("C706Ba9fa4fedF4507CB7A898b4766c1bbf9be57");

/// The address of the system contract on Devnet.
pub const DEV_SYSTEM_CONTRAT_ADDRESS: Address =
    address!("000000000000000000000000000000000003dead");

/// The L1 start block for Mainnet.
pub const MAINNET_L1_START_BLOCK_NUMBER: u64 = 18318215;

/// The L1 start block for Sepolia.
pub const SEPOLIA_L1_START_BLOCK_NUMBER: u64 = 4041343;

/// The L1 start block for Devnet.
pub const DEV_L1_START_BLOCK_NUMBER: u64 = 0;

/// A shared configuration for the node.
#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// The address book of relevant addresses for Scroll.
    pub address_book: ScrollAddressBook,
    /// The start block for the L1 sync.
    pub start_l1_block: u64,
}

/// An address book for Scroll.
#[derive(Debug, Clone, Default)]
pub struct ScrollAddressBook {
    /// The address of the rollup node contract.
    pub rollup_node_contract_address: Address,
    /// The address of the v1 message queue contract.
    pub v1_message_queue_address: Address,
    /// The address of the v2 message queue contract.
    pub v2_message_queue_address: Address,
    /// The address of the system contract used in consensus.
    pub system_contract_address: Address,
}

impl NodeConfig {
    /// Returns the node configuration for Mainnet.
    pub const fn mainnet() -> Self {
        Self {
            address_book: ScrollAddressBook {
                rollup_node_contract_address: MAINNET_ROLLUP_CONTRACT_ADDRESS,
                v1_message_queue_address: MAINNET_L1_MESSAGE_QUEUE_V1_CONTRACT_ADDRESS,
                v2_message_queue_address: MAINNET_L1_MESSAGE_QUEUE_V2_CONTRACT_ADDRESS,
                system_contract_address: MAINNET_SYSTEM_CONTRAT_ADDRESS,
            },
            start_l1_block: MAINNET_L1_START_BLOCK_NUMBER,
        }
    }

    /// Returns the node configuration for Sepolia.
    pub const fn sepolia() -> Self {
        Self {
            address_book: ScrollAddressBook {
                rollup_node_contract_address: SEPOLIA_ROLLUP_CONTRACT_ADDRESS,
                v1_message_queue_address: SEPOLIA_L1_MESSAGE_QUEUE_V1_CONTRACT_ADDRESS,
                v2_message_queue_address: SEPOLIA_L1_MESSAGE_QUEUE_V2_CONTRACT_ADDRESS,
                system_contract_address: SEPOLIA_SYSTEM_CONTRAT_ADDRESS,
            },
            start_l1_block: SEPOLIA_L1_START_BLOCK_NUMBER,
        }
    }

    /// Returns the node configuration for Devnet.
    pub const fn dev() -> Self {
        Self {
            address_book: ScrollAddressBook {
                rollup_node_contract_address: DEVNET_ROLLUP_CONTRACT_ADDRESS,
                v1_message_queue_address: DEVNET_L1_MESSAGE_QUEUE_V1_CONTRACT_ADDRESS,
                v2_message_queue_address: DEVNET_L1_MESSAGE_QUEUE_V2_CONTRACT_ADDRESS,
                system_contract_address: DEV_SYSTEM_CONTRAT_ADDRESS,
            },
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
