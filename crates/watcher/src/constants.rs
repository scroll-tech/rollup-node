use std::sync::LazyLock;

use alloy_primitives::{address, Address};
use alloy_rpc_types_eth::Filter;
use alloy_sol_types::SolEvent;
use scroll_l1::abi::logs::{CommitBatch, QueueTransaction};

/// The address of the Scroll Rollup contract on the L1.
pub const ROLLUP_CONTRACT_ADDRESS: Address = address!("0x2D567EcE699Eabe5afCd141eDB7A4f2D0D6ce8a0");

/// The address of the Scroll L1 message queue contract on the L1.
pub const L1_MESSAGE_QUEUE_CONTRACT_ADDRESS: Address =
    address!("0xCd81CCC2d3f20DCEa1740aDD7C4a56Fd08471009");

/// The [`Filter`] used by the [`crate::L1Watcher`] to index events relevant to the rollup node.
pub static L1_WATCHER_LOG_FILTER: LazyLock<Filter> = LazyLock::new(|| {
    Filter::new()
        .address(vec![ROLLUP_CONTRACT_ADDRESS, L1_MESSAGE_QUEUE_CONTRACT_ADDRESS])
        .event_signature(vec![QueueTransaction::SIGNATURE_HASH, CommitBatch::SIGNATURE_HASH])
});
