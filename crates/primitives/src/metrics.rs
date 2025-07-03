use core::{
    fmt::{Debug, Formatter},
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use std::time::Instant;

/// A metered future that records the start of the polling.
pub struct MeteredFuture<F> {
    /// The future being metered.
    pub fut: F,
    started_at: Instant,
}

impl<F> Debug for MeteredFuture<F> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MeteredFuture")
            .field("fut", &"Future")
            .field("started_at", &self.started_at)
            .finish()
    }
}

impl<F> MeteredFuture<F> {
    /// Returns a new instance of a [`MeteredFuture`].
    pub fn new(fut: F) -> Self {
        Self { fut, started_at: Instant::now() }
    }

    /// Adds the provided duration to the `started_at` field.
    pub fn with_initial_duration(mut self, initial_duration: Duration) -> Self {
        self.started_at =
            self.started_at.clone().checked_sub(initial_duration).unwrap_or(self.started_at);
        self
    }
}

impl<F: Future + Unpin> Future for MeteredFuture<F> {
    type Output = (Duration, F::Output);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let started_at = &self.started_at.clone();
        match Pin::new(&mut self.get_mut().fut).poll(cx) {
            Poll::Ready(output) => {
                let polling_duration = started_at.elapsed();
                Poll::Ready((polling_duration, output))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
