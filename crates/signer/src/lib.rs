//! A library responsible for signing artifacts for the rollup node.
//!
//! The signer is generic and can use any implementation of the `Signer` trait from the
//! `alloy_signer` crate, including local and remote signers such as AWS KMS.
//!
//! Currently it only supports signing L2 blocks, however it can be extended to
//! support signing other artifacts in the future such as pre-commitments.

use std::time::Instant;

use futures::stream::{FuturesOrdered, StreamExt};
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use tokio_stream::wrappers::UnboundedReceiverStream;

mod error;
pub use error::SignerError;

mod event;
pub use event::SignerEvent;

mod future;
pub use future::{sign_block, SignerFuture};

mod handle;
pub use handle::SignerHandle;

mod metrics;
pub use metrics::SignerMetrics;

mod requests;
pub use requests::SignerRequest;

/// The signer instance is responsible for signing artifacts for the rollup node.
pub struct Signer {
    /// The signer instance.
    signer: Arc<dyn alloy_signer::Signer + Send + Sync>,
    /// A stream of pending signing requests.
    requests: UnboundedReceiverStream<SignerRequest>,
    /// In progress signing requests.
    in_progress: FuturesOrdered<SignerFuture>,
    /// A channel to send events to the engine driver.
    sender: UnboundedSender<SignerEvent>,
    /// The signer metrics.
    metrics: SignerMetrics,
}

impl Signer {
    /// Creates a new [`Signer`] instance and [`SignerHandle`] with the provided signer.
    fn new(signer: impl alloy_signer::Signer + Send + Sync + 'static) -> (Self, SignerHandle) {
        let (req_tx, req_rx) = tokio::sync::mpsc::unbounded_channel();
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let signer = Self {
            signer: Arc::new(signer),
            requests: req_rx.into(),
            in_progress: FuturesOrdered::new(),
            sender: event_tx,
            metrics: SignerMetrics::default(),
        };
        (signer, SignerHandle::new(req_tx, event_rx.into()))
    }

    /// Spawns a new `Signer` instance onto the tokio runtime.
    pub fn spawn(signer: impl alloy_signer::Signer + Send + Sync + 'static) -> SignerHandle {
        let (signer, handle) = Self::new(signer);
        tokio::spawn(signer.run());
        handle
    }

    /// Execution loop for the signer.
    async fn run(mut self) {
        loop {
            tokio::select! {
                Some(request) = self.requests.next() => {
                    match request {
                        SignerRequest::SignBlock(block) => {
                            let signer = self.signer.clone();
                            let metric = self.metrics.clone();
                            let future = Box::pin(async move {
                                let now = Instant::now();
                                let res = sign_block(block, signer).await;
                                metric.signing_duration.set(now.elapsed().as_secs_f64());
                                res
                            });
                            self.in_progress.push_back(future);
                        }

                    }
                }
                Some(result) = self.in_progress.next(), if !self.in_progress.is_empty() => {
                    match result {
                        Ok(event) => {
                            if self.sender.send(event).is_err() {
                                tracing::info!(target: "scroll::signer", "The event channel has been closed - shutting down.");
                                break;
                            }
                        },
                        Err(err) => {
                            tracing::error!(target: "scroll::signer", ?err, "An error occurred while signing");
                        }
                    }
                }
                else => {
                    // The request channel is closed, exit the loop.
                    tracing::info!(target: "scroll::signer", "Signer request channel has been closed - shutting down.");
                    break;
                }
            }
        }
    }
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signer")
            .field("signer", &"alloy_signer::Signer")
            .field("requests", &self.requests)
            .field("in_progress", &self.in_progress)
            .field("sender", &self.sender)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_signer_local::PrivateKeySigner;
    use reth_scroll_primitives::ScrollBlock;
    use rollup_node_primitives::sig_encode_hash;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_signer_local() {
        reth_tracing::init_test_tracing();
        let signer = PrivateKeySigner::random();
        let mut handle = Signer::spawn(Box::new(signer.clone()));

        // Test sending a request
        let block = ScrollBlock::default();
        handle.sign_block(block.clone()).unwrap();

        // Test receiving an event
        let event = handle.next().await.unwrap();
        let (event_block, signature) = match event {
            SignerEvent::SignedBlock { block, signature } => (block, signature),
        };
        let hash = sig_encode_hash(&event_block);
        let recovered_address = signature.recover_address_from_prehash(&hash).unwrap();

        assert_eq!(event_block, block);
        assert_eq!(recovered_address, signer.address());
    }

    // The following tests do not have any assertions, they are just to ensure that the
    // shutdown logic works correctly and does not panic. You can observer the logs to see if the
    // shutdown logic is executed correctly.

    #[tokio::test]
    async fn test_drop_signer_handle() {
        reth_tracing::init_test_tracing();

        // Create a local signer and the signer service
        let key = PrivateKeySigner::random();
        let (signer, handle) = Signer::new(Box::new(key.clone()));

        // Spawn the signer task and capture the JoinHandle
        let task = tokio::spawn(signer.run());

        // Drop the handle to simulate shutdown
        drop(handle);

        // Wait for the signer task to complete
        task.await.expect("Signer task panicked");
    }

    #[tokio::test]
    async fn test_drop_signer_handle_with_wait() {
        reth_tracing::init_test_tracing();

        // Create a local signer and the signer service
        let key = PrivateKeySigner::random();
        let (signer, handle) = Signer::new(Box::new(key.clone()));

        // Spawn the signer task and capture the JoinHandle
        let task = tokio::spawn(signer.run());

        // wait to observe if the else block will be executed.
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Drop the handle to simulate shutdown
        drop(handle);

        // Wait for the signer task to complete
        task.await.expect("signer task panicked");
    }

    #[tokio::test]
    async fn test_drop_signer_handle_with_request() {
        reth_tracing::init_test_tracing();

        // Create a local signer and the signer service
        let key = PrivateKeySigner::random();
        let (signer, handle) = Signer::new(Box::new(key.clone()));

        // Spawn the signer task and capture the JoinHandle
        let task = tokio::spawn(signer.run());

        // Send a signing request through the handle
        let block = ScrollBlock::default();
        handle.sign_block(block.clone()).unwrap();

        // Drop the handle to simulate shutdown
        drop(handle);

        // Wait for the signer task to complete
        task.await.expect("Signer task panicked");
    }

    #[tokio::test]
    async fn test_signer_with_hex_key_file_with_prefix() {
        reth_tracing::init_test_tracing();

        // Create temp file with hex key (with 0x prefix)
        let mut temp_file = NamedTempFile::new().unwrap();
        let private_key_hex = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        temp_file.write_all(private_key_hex.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        // Simulate the logic from rollup.rs's key initialization based on the file content
        let key_content = std::fs::read_to_string(temp_file.path()).unwrap().trim().to_string();
        let hex_str = key_content.strip_prefix("0x").unwrap_or(&key_content);
        let key_bytes = hex::decode(hex_str).unwrap();
        let signer = PrivateKeySigner::from_slice(&key_bytes).unwrap();

        let mut handle = Signer::spawn(Box::new(signer.clone()));

        // Test signing
        let block = ScrollBlock::default();
        handle.sign_block(block.clone()).unwrap();

        // Verify signature
        let event = handle.next().await.unwrap();
        let (event_block, signature) = match event {
            SignerEvent::SignedBlock { block, signature } => (block, signature),
        };
        let hash = sig_encode_hash(&event_block);
        let recovered_address = signature.recover_address_from_prehash(&hash).unwrap();

        assert_eq!(event_block, block);
        assert_eq!(recovered_address, signer.address());
    }

    #[tokio::test]
    async fn test_signer_with_hex_key_file_without_prefix() {
        reth_tracing::init_test_tracing();

        // Create temp file with hex key (without 0x prefix)
        let mut temp_file = NamedTempFile::new().unwrap();
        let private_key_hex = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        temp_file.write_all(private_key_hex.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        // Simulate the logic from rollup.rs's key initialization based on the file content
        let key_content = std::fs::read_to_string(temp_file.path()).unwrap().trim().to_string();
        let hex_str = key_content.strip_prefix("0x").unwrap_or(&key_content);
        let key_bytes = hex::decode(hex_str).unwrap();
        let signer = PrivateKeySigner::from_slice(&key_bytes).unwrap();

        let mut handle = Signer::spawn(Box::new(signer.clone()));

        // Test signing
        let block = ScrollBlock::default();
        handle.sign_block(block.clone()).unwrap();

        // Verify signature
        let event = handle.next().await.unwrap();
        let (event_block, signature) = match event {
            SignerEvent::SignedBlock { block, signature } => (block, signature),
        };
        let hash = sig_encode_hash(&event_block);
        let recovered_address = signature.recover_address_from_prehash(&hash).unwrap();

        assert_eq!(event_block, block);
        assert_eq!(recovered_address, signer.address());
    }

    #[test]
    fn test_hex_key_file_invalid_content_case_1() {
        // Test invalid hex content
        let mut temp_file = NamedTempFile::new().unwrap();
        let invalid_hex = "invalid_hex_content";
        temp_file.write_all(invalid_hex.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let key_content = std::fs::read_to_string(temp_file.path()).unwrap().trim().to_string();
        let hex_str = key_content.strip_prefix("0x").unwrap_or(&key_content);
        let result = hex::decode(hex_str);

        assert!(result.is_err());

        let error = result.unwrap_err();
        assert!(error.to_string().contains("Odd number of digits"));
    }

    #[test]
    fn test_hex_key_file_invalid_content_case_2() {
        // Test invalid hex content
        let mut temp_file = NamedTempFile::new().unwrap();
        let invalid_hex = "invalid_hex_content_";
        temp_file.write_all(invalid_hex.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let key_content = std::fs::read_to_string(temp_file.path()).unwrap().trim().to_string();
        let hex_str = key_content.strip_prefix("0x").unwrap_or(&key_content);
        let result = hex::decode(hex_str);

        assert!(result.is_err());

        let error = result.unwrap_err();
        assert!(error.to_string().contains("Invalid character 'i' at position 0"));
    }

    #[test]
    fn test_hex_key_file_wrong_length() {
        // Test key with wrong length
        let mut temp_file = NamedTempFile::new().unwrap();
        let short_key = "0x1234"; // Too short
        temp_file.write_all(short_key.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let key_content = std::fs::read_to_string(temp_file.path()).unwrap().trim().to_string();
        let hex_str = key_content.strip_prefix("0x").unwrap_or(&key_content);
        let key_bytes = hex::decode(hex_str).unwrap();
        let result = PrivateKeySigner::from_slice(&key_bytes);

        assert!(result.is_err());

        let error = result.unwrap_err();
        assert!(error.to_string().contains("signature error"));
    }
}
