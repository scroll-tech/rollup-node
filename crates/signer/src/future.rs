use super::{SignerError, SignerEvent};
use reth_scroll_primitives::ScrollBlock;
use std::{future::Future, pin::Pin, sync::Arc};

/// A type alias for a future that resolves to a `SignerEvent` or a `SignerError`.
pub type SignerFuture = Pin<Box<dyn Future<Output = Result<SignerEvent, SignerError>> + Send>>;

/// A future that signs a block using the provided signer.
pub fn sign_block(
    block: ScrollBlock,
    signer: Arc<Box<dyn alloy_signer::Signer + Send + Sync>>,
) -> SignerFuture {
    Box::pin(async move {
        let signature = signer.sign_hash(&block.hash_slow()).await?;
        Ok(SignerEvent::SignedBlock((block, signature)))
    })
}
