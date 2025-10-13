use crate::{service::DatabaseService, BoxedFuture, DatabaseError, DatabaseQuery};
use metrics::Histogram;
use metrics_derive::Metrics;
use std::{fmt::Debug, time::Duration};
use tower::Service;

/// A type used for retrying transient failures in operations.
#[derive(Debug, Clone)]
pub struct Retry<S> {
    /// The inner service.
    inner: S,
    /// Maximum number of retry attempts. None means infinite retries
    pub max_retries: Option<usize>,
    /// Initial delay between retries in milliseconds
    pub initial_delay_ms: u64,
    /// Whether to use exponential backoff
    pub exponential_backoff: bool,
    /// Retry metrics.
    metrics: RetryMetrics,
}

/// Metrics for the retry service.
#[derive(Metrics, Clone)]
#[metrics(scope = "database_retry")]
struct RetryMetrics {
    /// Number of database query attempts before a successful result.
    #[metrics(describe = "Number of attempts before successful database query result")]
    pub attempts_before_query_success: Histogram,
}

impl<S> Retry<S> {
    /// Creates a new [`Retry`] with the specified parameters.
    pub fn new(
        inner: S,
        max_retries: Option<usize>,
        initial_delay_ms: u64,
        exponential_backoff: bool,
    ) -> Self {
        Self {
            inner,
            max_retries,
            initial_delay_ms,
            exponential_backoff,
            metrics: RetryMetrics::default(),
        }
    }

    /// Creates a new [`Retry`] with default retry parameters.
    pub fn new_with_default_config(inner: S) -> Self {
        Self::new(inner, None, 50, false)
    }
}

impl<T, Err, S> Service<DatabaseQuery<T, Err>> for Retry<S>
where
    S: DatabaseService + Clone + Send + 'static,
    T: Send + 'static,
    Err: From<DatabaseError> + CanRetry + Debug + Send + 'static,
{
    type Response = T;
    type Error = Err;
    type Future = BoxedFuture<Self::Response, Self::Error>;

    fn poll_ready(
        &mut self,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: DatabaseQuery<T, Err>) -> Self::Future {
        let mut inner = self.inner.clone();
        let this = self.clone();

        Box::pin(async move {
            let mut attempt: usize = 0;

            loop {
                match inner.call(req.clone()).await {
                    Ok(result) => {
                        this.metrics.attempts_before_query_success.record(attempt as f64);
                        return Ok(result)
                    }
                    Err(error) => {
                        // If the error is not retryable, return immediately.
                        if !error.can_retry() {
                            return Err(error);
                        }

                        if let Some(max_retries) = this.max_retries {
                            if attempt >= max_retries {
                                return Err(error);
                            }
                        }

                        // Calculate delay for next retry
                        let delay_ms = if this.exponential_backoff {
                            this.initial_delay_ms * 2_u64.pow(attempt as u32 - 1)
                        } else {
                            this.initial_delay_ms
                        };

                        attempt += 1;
                        tracing::debug!(target: "scroll::chain_orchestrator", ?error, attempt, delay_ms, "Retrying database query");

                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    }
                }
            }
        })
    }
}

#[async_trait::async_trait]
impl<S: DatabaseService> DatabaseService for Retry<S> {
    async fn call<
        T: Send + 'static,
        Err: From<DatabaseError> + CanRetry + Debug + Send + 'static,
    >(
        &mut self,
        req: DatabaseQuery<T, Err>,
    ) -> Result<T, Err> {
        Service::call(self, req).await
    }
}

/// A trait for errors that can indicate whether an operation can be retried.
pub trait CanRetry {
    /// Returns true if the implementer can be retried.
    fn can_retry(&self) -> bool;
}

// Centralized retry classification impls
impl CanRetry for DatabaseError {
    fn can_retry(&self) -> bool {
        matches!(self, Self::DatabaseError(_) | Self::SqlxError(_))
    }
}
