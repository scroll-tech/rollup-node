//! Contains common helper functions for integration tests.

use eyre::Result;
use std::{future::Future, time::Duration};

/// Retries an async operation with exponential backoff.
pub async fn retry_operation<F, Fut, T, E>(mut operation: F, max_attempts: usize) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut last_error = None;
    for attempt in 1..=max_attempts {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                println!("❌ Attempt {attempt} failed: {e}");
                last_error = Some(e);
                if attempt < max_attempts {
                    let delay = Duration::from_secs(attempt as u64 * 2);
                    println!("⏳ Waiting {delay:?} before retry...");
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
    Err(last_error.unwrap())
}
