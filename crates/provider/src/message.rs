use scroll_primitives::L1Message;
use std::ops::Range;

/// A provider for L1 messages.
pub trait L1MessageProvider {
    /// Get the L1 message with the given queue index.
    fn get_l1_message(&self, queue_index: u64) -> Option<L1Message>;

    /// Get the L1 messages within the given queue index range.
    fn get_l1_message_range(&self, queue_index_rang: Range<u64>) -> Vec<L1Message>;

    /// Insert an L1 message into the provider.
    fn insert_l1_message(&self, l1_message: L1Message);
}
