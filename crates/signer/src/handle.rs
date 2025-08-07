use alloy_primitives::Address;
use futures::{stream::StreamExt, Stream};
use tokio::sync::mpsc::UnboundedSender;
use tokio_stream::wrappers::UnboundedReceiverStream;

use super::{SignerError, SignerEvent, SignerRequest};

/// A handler for the signer that allows sending requests and receiving events.
#[derive(Debug)]
pub struct SignerHandle {
    /// A channel to send requests to the signer.
    pub request_tx: UnboundedSender<SignerRequest>,
    /// A channel to receive events from the signer.
    pub event_rx: UnboundedReceiverStream<SignerEvent>,
    /// The signer address.
    pub address: Address,
}

impl SignerHandle {
    /// Creates a new [`SignerHandle`] instance.
    pub const fn new(
        request_tx: UnboundedSender<SignerRequest>,
        event_rx: UnboundedReceiverStream<SignerEvent>,
        address: Address,
    ) -> Self {
        Self { request_tx, event_rx, address }
    }

    /// Sends a request to sign a block.
    pub fn sign_block(
        &self,
        block: reth_scroll_primitives::ScrollBlock,
    ) -> Result<(), SignerError> {
        self.request_tx
            .send(SignerRequest::SignBlock(block))
            .map_err(|_| SignerError::RequestChannelClosed)?;
        Ok(())
    }
}

impl Stream for SignerHandle {
    type Item = SignerEvent;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.event_rx.poll_next_unpin(cx)
    }
}
