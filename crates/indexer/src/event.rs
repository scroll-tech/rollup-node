/// An event emitted by the indexer.
#[derive(Debug, Clone, Copy)]
pub enum IndexerEvent {
    /// An event indicating that an L1Notification has been indexed.
    L1NotificationIndexed,
}
