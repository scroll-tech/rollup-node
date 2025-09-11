use super::{SignerError, SignerEvent};
use std::{future::Future, pin::Pin, sync::Arc};

use reth_scroll_primitives::ScrollBlock;
use rollup_node_primitives::sig_encode_hash;

/// A type alias for a future that resolves to a `SignerEvent` or a `SignerError`.
pub type SignerFuture = Pin<Box<dyn Future<Output = Result<SignerEvent, SignerError>> + Send>>;

/// A future that signs a block using the provided signer.
pub fn sign_block(
    block: ScrollBlock,
    signer: Arc<dyn alloy_signer::Signer + Send + Sync>,
) -> SignerFuture {
    Box::pin(async move {
        // TODO: Are we happy to sign the hash directly or do we want to use EIP-191
        // (`signer.sign_message`)?
        let hash = sig_encode_hash(&block.header);
        let signature = signer.sign_hash(&hash).await?;
        Ok(SignerEvent::SignedBlock { block, signature })
    })
}
