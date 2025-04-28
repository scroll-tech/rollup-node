//! A library responsible for signing artifacts for the rollup node.
//!
//! The signer is generic and can use any implementation of the `Signer` trait from the
//! `alloy_signer` crate, including local and remote signers such as AWS KMS.
//!
//! Currently it only supports signing L2 blocks, however it can be extended to
//! support signing other artifacts in the future such as pre-commitments.

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

mod requests;
pub use requests::SignerRequest;

/// The signer instance is responsible for signing artifacts for the rollup node.
pub struct Signer {
    // The signer instance.
    signer: Arc<Box<dyn alloy_signer::Signer + Send + Sync>>,
    // A stream of pending signing requests.
    requests: UnboundedReceiverStream<SignerRequest>,
    // In progress signing requests.
    in_progress: FuturesOrdered<SignerFuture>,
    /// A channel to send events to the engine driver.
    sender: UnboundedSender<SignerEvent>,
}

impl Signer {
    /// Creates a new `Signer` instance.
    pub async fn spawn(signer: Box<dyn alloy_signer::Signer + Send + Sync>) -> SignerHandle {
        let (req_tx, req_rx) = tokio::sync::mpsc::unbounded_channel();
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let signer = Self {
            signer: Arc::new(signer),
            requests: req_rx.into(),
            in_progress: FuturesOrdered::new(),
            sender: event_tx,
        };
        tokio::spawn(signer.run());
        SignerHandle::new(req_tx, event_rx.into())
    }

    /// Execution loop for the signer.
    async fn run(mut self) {
        loop {
            tokio::select! {
                Some(request) = self.requests.next() => {
                    match request {
                        SignerRequest::SignBlock(block) => {
                            let signer = self.signer.clone();
                            let future = sign_block(block, signer);
                            self.in_progress.push_back(future);
                        }

                    }
                }
                Some(result) = self.in_progress.next() => {
                    match result {
                        Ok(event) => self.sender.send(event).expect("The event channel is closed"),
                        Err(err) => {
                            tracing::error!(target: "rollup_node::signer", ?err, "An error occurred while signing");
                        }
                    }

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

    #[tokio::test]
    async fn test_signer_local() {
        let signer = PrivateKeySigner::random();
        let mut handle = Signer::spawn(Box::new(signer.clone())).await;

        // Test sending a request
        let block = ScrollBlock::default();
        handle.sign_block(block).unwrap();

        // Test receiving an event
        let event = handle.next().await.unwrap();
        let (block, signature) = match event {
            SignerEvent::SignedBlock { block, signature } => (block, signature),
        };
        let recovered_address = signature.recover_address_from_prehash(&block.hash_slow()).unwrap();

        assert_eq!(block, block);
        assert_eq!(recovered_address, signer.address());
    }
}
