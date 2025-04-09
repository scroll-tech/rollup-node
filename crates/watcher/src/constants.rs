use std::sync::LazyLock;

use alloy_primitives::{address, Address};
use alloy_rpc_types_eth::Filter;
use alloy_sol_types::SolEvent;
use scroll_l1::abi::logs::{CommitBatch, QueueTransaction};

/// The address of the Scroll Rollup contract on the L1.
pub const ROLLUP_CONTRACT_ADDRESS: Address = address!("0xa13BAF47339d63B743e7Da8741db5456DAc1E556");

/// The address of the Scroll L1 message queue contract on the L1.
pub const L1_MESSAGE_QUEUE_CONTRACT_ADDRESS: Address =
    address!("0x0d7E906BD9cAFa154b048cFa766Cc1E54E39AF9B");

/// The [`Filter`] used by the [`crate::L1Watcher`] to index events relevant to the rollup node.
pub static L1_WATCHER_LOG_FILTER: LazyLock<Filter> = LazyLock::new(|| {
    Filter::new()
        .address(vec![ROLLUP_CONTRACT_ADDRESS, L1_MESSAGE_QUEUE_CONTRACT_ADDRESS])
        .event_signature(vec![QueueTransaction::SIGNATURE_HASH, CommitBatch::SIGNATURE_HASH])
});
