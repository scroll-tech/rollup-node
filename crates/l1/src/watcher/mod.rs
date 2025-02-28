use futures::Stream;

mod observation;
pub use observation::L1Event;

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
