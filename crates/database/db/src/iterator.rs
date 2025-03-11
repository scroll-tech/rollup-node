use super::models;
use futures::{Stream, StreamExt};
use scroll_primitives::{BatchInput, L1Message};
use sea_orm::DbErr;

/// A database entry stream that can be used to iterate over database entries.
#[async_trait::async_trait]
pub trait DatabaseEntryStream<DatabaseEntry>: Stream<Item = Result<DatabaseEntry, DbErr>> {
    /// The type of the stream item.
    type StreamItem: From<DatabaseEntry> + std::fmt::Debug;

    /// Get the next entry from the stream.
    async fn next_entry(&mut self) -> Option<Result<Self::StreamItem, DbErr>>;
}

#[async_trait::async_trait]
impl<T> DatabaseEntryStream<models::batch_input::Model> for T
where
    T: Stream<Item = Result<models::batch_input::Model, DbErr>> + Unpin + Send,
{
    type StreamItem = BatchInput;

    async fn next_entry(&mut self) -> Option<Result<Self::StreamItem, DbErr>> {
        let next = self.next().await;
        next.map(|x| x.map(Into::into))
    }
}

#[async_trait::async_trait]
impl<T> DatabaseEntryStream<models::l1_message::Model> for T
where
    T: Stream<Item = Result<models::l1_message::Model, DbErr>> + Unpin + Send,
{
    type StreamItem = L1Message;

    async fn next_entry(&mut self) -> Option<Result<Self::StreamItem, DbErr>> {
        let next = self.next().await;
        next.map(|x| x.map(Into::into))
    }
}
