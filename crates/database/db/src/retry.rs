//! Retry mechanism for database operations

use std::time::Duration;

/// Configuration for retry behavior
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts. None means infinite retries
    pub max_retries: Option<usize>,
    /// Initial delay between retries in milliseconds
    pub initial_delay_ms: u64,
    /// Whether to use exponential backoff
    pub exponential_backoff: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: None,
            initial_delay_ms: 50,
            exponential_backoff: false,
        }
    }
}

/// Helper function to create a retry configuration
pub const fn retry_config(max_retries: usize, initial_delay_ms: u64, exponential_backoff: bool) -> RetryConfig {
    RetryConfig {
        max_retries: Some(max_retries),
        initial_delay_ms,
        exponential_backoff,
    }
}

/// Retry a database operation with operation name for better logging
pub async fn retry_operation_with_name<F, Fut, T, E>(
    operation_name: &str,
    operation: F,
    config: RetryConfig,
) -> Result<T, E>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Debug,
{
    let mut attempt = 0;
    
    loop {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(error) => {
                if let Some(max_retries) = config.max_retries {
                    if attempt >= max_retries {
                        return Err(error);
                    }
                }
                
                attempt += 1;
                tracing::debug!(
                    target: "scroll::db",
                    operation = operation_name,
                    error = ?error,
                    attempt = attempt,
                    "Retrying database operation"
                );
                
                // Calculate delay for next retry
                let delay_ms = if config.exponential_backoff {
                    config.initial_delay_ms * 2_u64.pow(attempt as u32 - 1)
                } else {
                    config.initial_delay_ms
                };
                
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
        }
    }
}

/// Convenience function for retrying with default configuration
pub async fn retry_with_defaults<F, Fut, T, E>(operation_name: &str, operation: F) -> Result<T, E>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Debug,
{
    retry_operation_with_name(operation_name, operation, RetryConfig::default()).await
}

#[cfg(test)]
mod tests {
    use crate::{retry_config, retry_operation_with_name, retry_with_defaults};
    use std::cell::RefCell;

    #[tokio::test]
    async fn test_retry_success_on_first_attempt() {
        let attempt = RefCell::new(0);
        let result = retry_operation_with_name(
            "test_operation",
            || {
                *attempt.borrow_mut() += 1;
                async move { Ok::<i32, &str>(42) }
            },
            retry_config(3, 10, false),
        ).await;

        assert_eq!(result, Ok(42));
        assert_eq!(*attempt.borrow(), 1);
    }

    #[tokio::test]
    async fn test_retry_success_after_failures() {
        let attempt = RefCell::new(0);
        let result = retry_operation_with_name(
            "test_operation",
            || {
                *attempt.borrow_mut() += 1;
                let current_attempt = *attempt.borrow();
                async move {
                    if current_attempt < 3 {
                        Err::<i32, &str>("failed")
                    } else {
                        Ok(42)
                    }
                }
            },
            retry_config(5, 10, false),
        ).await;

        assert_eq!(result, Ok(42));
        assert_eq!(*attempt.borrow(), 3);
    }

    #[tokio::test]
    async fn test_retry_exhausted() {
        let attempt = RefCell::new(0);
        let result = retry_operation_with_name(
            "test_operation",
            || {
                *attempt.borrow_mut() += 1;
                async move { Err::<i32, &str>("always fails") }
            },
            retry_config(2, 10, false),
        ).await;

        assert_eq!(result, Err("always fails"));
        assert_eq!(*attempt.borrow(), 3); // 1 initial + 2 retries
    }

    #[tokio::test]
    async fn test_retry_with_defaults() {
        let attempt = RefCell::new(0);
        let result = retry_with_defaults("test_retry_with_defaults", || {
            *attempt.borrow_mut() += 1;
            let current_attempt = *attempt.borrow();
            async move {
                if current_attempt < 2 {
                    Err::<i32, &str>("failed")
                } else {
                    Ok(42)
                }
            }
        }).await;

        assert_eq!(result, Ok(42));
        assert_eq!(*attempt.borrow(), 2);
    }
}
