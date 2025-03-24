//! A stateless derivation pipeline for Scroll.
//!
//! This crate provides a simple implementation of a derivation pipeline that transforms commit
//! payload into payload attributes for block building.

pub use error::DerivationPipelineError;
mod error;

pub use hash::try_compute_data_hash;
mod hash;

use alloy_primitives::B256;
use alloy_rpc_types_engine::PayloadAttributes;
use reth_scroll_chainspec::SCROLL_FEE_VAULT_ADDRESS;
use scroll_alloy_consensus::TxL1Message;
use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;
use scroll_codec::decoding::batch::Batch;

/// An instance of the trait can be used to provide the next L1 message to be used in the derivation
/// pipeline.
pub trait L1MessageProvider {
    /// Returns the next L1 message.
    fn next_l1_message(&self) -> TxL1Message;
}

/// Returns an iterator over [`ScrollPayloadAttributes`] from the [`Batch`] and a
/// [`L1MessageProvider`].
pub fn derive<P: L1MessageProvider>(
    batch: Batch,
    l1_message_provider: &P,
) -> Result<impl Iterator<Item = ScrollPayloadAttributes> + use<'_, P>, DerivationPipelineError> {
    let iter = batch.data.into_l2_blocks().into_iter().map(|block| {
        let mut txs = (0..block.context.num_l1_messages)
            .map(|_| l1_message_provider.next_l1_message())
            .map(|tx| {
                let mut bytes = Vec::new();
                tx.eip2718_encode(&mut bytes);
                bytes.into()
            })
            .collect::<Vec<_>>();
        let mut l2_txs = block.transactions.clone();
        txs.append(&mut l2_txs);

        ScrollPayloadAttributes {
            payload_attributes: PayloadAttributes {
                timestamp: block.context.timestamp,
                prev_randao: B256::ZERO,
                suggested_fee_recipient: SCROLL_FEE_VAULT_ADDRESS,
                withdrawals: None,
                parent_beacon_block_root: None,
            },
            transactions: Some(txs),
            no_tx_pool: true,
        }
    });

    Ok(iter)
}
