//! Configurable retry mechanism for database, network, and other fallible operations.

use std::time::Duration;

/// A type used for retrying transient failures in operations.
#[derive(Debug, Clone)]
pub struct Retry {
    /// Maximum number of retry attempts. None means infinite retries
    pub max_retries: Option<usize>,
    /// Initial delay between retries in milliseconds
    pub initial_delay_ms: u64,
    /// Whether to use exponential backoff
    pub exponential_backoff: bool,
}

impl Default for Retry {
    fn default() -> Self {
        Self { max_retries: None, initial_delay_ms: 50, exponential_backoff: false }
    }
}

impl Retry {
    /// Creates a new [`Retry`] with the specified parameters.
    pub const fn new(
        max_retries: Option<usize>,
        initial_delay_ms: u64,
        exponential_backoff: bool,
    ) -> Self {
        Self { max_retries, initial_delay_ms, exponential_backoff }
    }

    /// Retry an asynchronous operation with the configured retry strategy.
    pub async fn retry<F, Fut, T, E>(&self, operation_name: &str, operation: F) -> Result<T, E>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
        E: std::fmt::Debug,
    {
        let mut attempt: usize = 0;

        loop {
            match operation().await {
                Ok(result) => return Ok(result),
                Err(error) => {
                    if let Some(max_retries) = self.max_retries {
                        if attempt >= max_retries {
                            return Err(error);
                        }
                    }

                    attempt += 1;
                    tracing::debug!(
                        target: "scroll::chain_orchestrator",
                        operation = operation_name,
                        error = ?error,
                        attempt = attempt,
                        "Retrying operation"
                    );

                    // Calculate delay for next retry
                    let delay_ms = if self.exponential_backoff {
                        self.initial_delay_ms * 2_u64.pow(attempt as u32 - 1)
                    } else {
                        self.initial_delay_ms
                    };

                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Retry;
    use std::cell::RefCell;

    #[tokio::test]
    async fn test_retry_success_on_first_attempt() {
        let attempt = RefCell::new(0);
        let retry = Retry::new(Some(3), 10, false);
        let result = retry
            .retry("test_operation", || {
                *attempt.borrow_mut() += 1;
                async move { Ok::<i32, &str>(42) }
            })
            .await;

        assert_eq!(result, Ok(42));
        assert_eq!(*attempt.borrow(), 1);
    }

    #[tokio::test]
    async fn test_retry_success_after_failures() {
        let attempt = RefCell::new(0);
        let retry = Retry::new(Some(5), 10, false);
        let result = retry
            .retry("test_operation", || {
                *attempt.borrow_mut() += 1;
                let current_attempt = *attempt.borrow();
                async move {
                    if current_attempt < 3 {
                        Err::<i32, &str>("failed")
                    } else {
                        Ok(42)
                    }
                }
            })
            .await;

        assert_eq!(result, Ok(42));
        assert_eq!(*attempt.borrow(), 3);
    }

    #[tokio::test]
    async fn test_retry_exhausted() {
        let attempt = RefCell::new(0);
        let retry = Retry::new(Some(2), 10, false);
        let result = retry
            .retry("test_operation", || {
                *attempt.borrow_mut() += 1;
                async move { Err::<i32, &str>("always fails") }
            })
            .await;

        assert_eq!(result, Err("always fails"));
        assert_eq!(*attempt.borrow(), 3); // 1 initial + 2 retries
    }

    #[tokio::test]
    async fn test_retry_with_defaults() {
        let attempt = RefCell::new(0);
        let retry = Retry::default();
        let result = retry
            .retry("test_retry_with_defaults", || {
                *attempt.borrow_mut() += 1;
                let current_attempt = *attempt.borrow();
                async move {
                    if current_attempt < 2 {
                        Err::<i32, &str>("failed")
                    } else {
                        Ok(42)
                    }
                }
            })
            .await;

        assert_eq!(result, Ok(42));
        assert_eq!(*attempt.borrow(), 2);
    }
}
