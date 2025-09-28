use crate::database::SingleOrMultiple;
use crate::request::DatabaseRequest;
use crate::DatabaseError;

use sea_orm::EntityTrait;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::time::sleep;
use tower::{Layer, Service};

#[derive(Debug, Default, Clone)]
pub struct RetryBackoffLayer {
    /// The maximum number of retries for rate limit errors
    max_rate_limit_retries: u32,
    /// The initial backoff in milliseconds
    initial_backoff: u64,
}

impl<S> Layer<S> for RetryBackoffLayer {
    type Service = RetryBackoffService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RetryBackoffService {
            inner,
            max_rate_limit_retries: self.max_rate_limit_retries,
            initial_backoff: self.initial_backoff,
            requests_enqueued: Arc::new(Default::default()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RetryBackoffService<S> {
    /// The inner service
    inner: S,
    /// The maximum number of retries for rate limit errors
    max_rate_limit_retries: u32,
    /// The initial backoff in milliseconds
    initial_backoff: u64,
    /// The number of requests currently enqueued
    requests_enqueued: Arc<AtomicU32>,
}

impl<T: EntityTrait, S> Service<DatabaseRequest<T>> for RetryBackoffService<S>
where
    S: Service<
            DatabaseRequest<T>,
            Future = Pin<
                Box<dyn Future<Output = Result<SingleOrMultiple<T::Model>, DatabaseError>> + Send>,
            >,
            Error = DatabaseError,
        > + Clone
        + Send
        + 'static,
{
    type Response = SingleOrMultiple<T::Model>;
    type Error = DatabaseError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: DatabaseRequest<T>) -> Self::Future {
        let mut inner = self.inner.clone();
        Box::pin(async move {
            loop {
                let res = inner.call(req.clone()).await;

                if let Ok(x) = res {
                    return Ok(x);
                }

                // TODO: implement correct back off and retry logic here.
                sleep(Duration::from_millis(1)).await;
            }
        })
    }
}
