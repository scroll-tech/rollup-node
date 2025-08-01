use super::L1Event;
use futures::Stream;

/// A watcher for observing events from the L1.
#[derive(Debug)]
pub struct L1Watcher;

impl Stream for L1Watcher {
    type Item = L1Event;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        todo!()
    }
}
