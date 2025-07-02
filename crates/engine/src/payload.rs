use alloy_primitives::{B256, B64};
use reth_primitives_traits::{AlloyBlockHeader, Block, BlockBody};
use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;

use tracing::debug;

/// Returns true if the [`Block`] matches the [`ScrollPayloadAttributes`]:
///    - parent hash matches the parent hash of the [`Block`].
///    - all transactions match.
///    - timestamps are equal.
///    - `prev_randaos` are equal.
///    - `block_data_hint` matches the block data if present.
pub(crate) fn block_matches_attributes<B: Block>(
    attributes: &ScrollPayloadAttributes,
    block: &B,
    parent_hash: B256,
) -> bool {
    let header = block.header();
    if header.parent_hash() != parent_hash {
        debug!(
            target: "scroll::engine::driver",
            expected = ?parent_hash,
            got = ?header.parent_hash(),
            "reorg: mismatch in parent hash"
        );
        return false;
    }

    let payload_transactions = &block.body().encoded_2718_transactions();
    let matching_transactions =
        attributes.transactions.as_ref().is_some_and(|v| v == payload_transactions);

    if !matching_transactions {
        debug!(
            target: "scroll::engine::driver",
            expected = ?attributes.transactions,
            got = ?payload_transactions,
            "reorg: mismatch in transactions"
        );
        return false;
    }

    if header.timestamp() != attributes.payload_attributes.timestamp {
        debug!(
            target: "scroll::engine::driver",
            expected = ?attributes.payload_attributes.timestamp,
            got = ?header.timestamp(),
            "reorg: mismatch in timestamp"
        );
        return false;
    }

    if header.mix_hash().unwrap_or_default() != attributes.payload_attributes.prev_randao {
        debug!(
            target: "scroll::engine::driver",
            expected = ?attributes.payload_attributes.prev_randao,
            got = ?header.mix_hash().unwrap_or_default(),
            "reorg: mismatch in prev_randao"
        );
        return false;
    }

    let block_data = &attributes.block_data_hint;
    if block_data.extra_data.as_ref().is_some_and(|ex| ex != header.extra_data()) {
        debug!(
            target: "scroll::engine::driver",
            expected = ?block_data.extra_data,
            got = ?header.extra_data(),
            "reorg: mismatch in extra_data"
        );
        return false;
    }
    if block_data.state_root.is_some_and(|d| d != header.state_root()) {
        debug!(
            target: "scroll::engine::driver",
            expected = ?block_data.state_root,
            got = ?header.state_root(),
            "reorg: mismatch in state_root"
        );
        return false;
    }
    if block_data.coinbase.is_some_and(|d| d != header.beneficiary()) {
        debug!(
            target: "scroll::engine::driver",
            expected = ?block_data.coinbase,
            got = ?header.beneficiary(),
            "reorg: mismatch in coinbase"
        );
        return false;
    }

    // nonce defaults to `Some(0x0000000000000000)` for `ScrollBlock`.
    if B64::from(block_data.nonce.unwrap_or_default()) != header.nonce().unwrap_or_default() {
        debug!(
            target: "scroll::engine::driver",
            expected = ?block_data.nonce,
            got = ?header.nonce(),
            "reorg: mismatch in nonce"
        );
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    use alloy_consensus::Header;
    use alloy_eips::Encodable2718;
    use alloy_primitives::{Bytes, U256};
    use arbitrary::{Arbitrary, Unstructured};
    use reth_scroll_primitives::ScrollBlock;
    use reth_testing_utils::{generators, generators::Rng};
    use scroll_alloy_consensus::ScrollTxEnvelope;
    use scroll_alloy_rpc_types_engine::BlockDataHint;

    #[test]
    fn test_matching_payloads() -> eyre::Result<()> {
        let mut bytes = [0u8; 1024];
        generators::rng().fill(bytes.as_mut_slice());
        let mut unstructured = Unstructured::new(&bytes);

        let parent_hash = B256::arbitrary(&mut unstructured)?;
        let transactions = Vec::<ScrollTxEnvelope>::arbitrary(&mut unstructured)?;
        let encoded_transactions = transactions
            .clone()
            .into_iter()
            .map(|tx| tx.encoded_2718().into())
            .collect::<Vec<Bytes>>();
        let prev_randao = B256::arbitrary(&mut unstructured)?;
        let timestamp = u64::arbitrary(&mut unstructured)?;
        let block_data_hint = BlockDataHint::arbitrary(&mut unstructured)?;

        let mut attributes = ScrollPayloadAttributes::arbitrary(&mut unstructured)?;
        attributes.transactions = Some(encoded_transactions);
        attributes.payload_attributes.timestamp = timestamp;
        attributes.payload_attributes.prev_randao = prev_randao;
        attributes.block_data_hint = block_data_hint.clone();

        let block = ScrollBlock {
            header: Header {
                parent_hash,
                timestamp,
                difficulty: block_data_hint.difficulty.unwrap_or_default(),
                nonce: block_data_hint.nonce.unwrap_or_default().into(),
                beneficiary: block_data_hint.coinbase.unwrap_or_default(),
                extra_data: block_data_hint.extra_data.unwrap_or_default(),
                state_root: block_data_hint.state_root.unwrap_or_default(),
                mix_hash: prev_randao,
                ..Default::default()
            },
            body: alloy_consensus::BlockBody { transactions, ..Default::default() },
        };

        assert!(block_matches_attributes(&attributes, &block, parent_hash));

        Ok(())
    }

    #[test]
    fn test_mismatched_payloads() -> eyre::Result<()> {
        let mut bytes = [0u8; 1024];
        generators::rng().fill(bytes.as_mut_slice());
        let mut unstructured = Unstructured::new(&bytes);

        let parent_hash = B256::arbitrary(&mut unstructured)?;
        let transactions = Vec::<ScrollTxEnvelope>::arbitrary(&mut unstructured)?;
        let prev_randao = B256::arbitrary(&mut unstructured)?;
        let timestamp = u64::arbitrary(&mut unstructured)?;
        let difficulty = U256::arbitrary(&mut unstructured)?;
        let extra_data = Bytes::arbitrary(&mut unstructured)?;

        let attributes = ScrollPayloadAttributes::arbitrary(&mut unstructured)?;
        let block = ScrollBlock {
            header: Header {
                parent_hash,
                timestamp,
                difficulty,
                extra_data,
                mix_hash: prev_randao,
                ..Default::default()
            },
            body: alloy_consensus::BlockBody { transactions, ..Default::default() },
        };

        assert!(!block_matches_attributes(&attributes, &block, parent_hash));

        Ok(())
    }
}
