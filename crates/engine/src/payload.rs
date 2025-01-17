use alloy_eips::BlockId;
use alloy_primitives::{Bytes, B256};
use alloy_rpc_types_engine::{ExecutionPayload, PayloadAttributes};
use eyre::Result;
use serde::{Deserialize, Serialize};
use std::future::Future;
use tracing::debug;

/// The payload attributes for block building tailored for Scroll.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScrollPayloadAttributes {
    /// The payload attributes.
    pub(crate) payload_attributes: PayloadAttributes,
    /// An optional array of transaction to be forced included in the block (includes l1 messages).
    pub(crate) transactions: Option<Vec<Bytes>>,
    /// Indicates whether the payload building job should happen with or without pool transactions.
    pub(crate) no_tx_pool: bool,
}

/// Returns true if the [`ScrollPayloadAttributes`] matches the [`ExecutionPayload`]:
///    - provided parent hash matches the parent hash of the [`ExecutionPayload`]
///    - all transactions match
///    - timestamps are equal
///    - `prev_randaos` are equal
///    - TODO: should we also compare the `fee_recipient` with the `suggested_fee_recipient`?
pub(crate) fn matching_payloads(
    attributes: &ScrollPayloadAttributes,
    payload: &ExecutionPayload,
    parent_hash: B256,
) -> bool {
    if payload.parent_hash() != parent_hash {
        debug!(
            target: "engine::driver",
            expected = ?parent_hash,
            got = ?payload.parent_hash(),
            "mismatch in parent hash"
        );
        return false
    }

    let payload_transactions = &payload.as_v1().transactions;
    let matching_transactions = payload_transactions.len() ==
        attributes.transactions.as_ref().map(|v| v.len()).unwrap_or_default() &&
        attributes.transactions.as_ref().is_some_and(|v| v == payload_transactions);

    if !matching_transactions {
        debug!(
            target: "engine::driver",
            expected = ?attributes.transactions,
            got = ?payload_transactions,
            "mismatch in transactions"
        );
        return false
    }

    if payload.timestamp() != attributes.payload_attributes.timestamp {
        debug!(
            target: "engine::driver",
            expected = ?attributes.payload_attributes.timestamp,
            got = ?payload.timestamp(),
            "mismatch in timestamp"
        );
        return false
    }

    if payload.prev_randao() != attributes.payload_attributes.prev_randao {
        debug!(
            target: "engine::driver",
            expected = ?attributes.payload_attributes.prev_randao,
            got = ?payload.prev_randao(),
            "mismatch in prev_randao"
        );
        return false
    }

    true
}

/// Implementers of the trait can provide the L2 execution payload for a block id.
pub trait ExecutionPayloadProvider {
    /// Returns the [`ExecutionPayload`] for the provided [`BlockId`], or [None].
    fn execution_payload_by_block(
        &self,
        block_id: BlockId,
    ) -> impl Future<Output = Result<Option<ExecutionPayload>>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_rpc_types_engine::ExecutionPayloadV1;
    use arbitrary::{Arbitrary, Unstructured};
    use reth_testing_utils::{generators, generators::Rng};

    fn default_execution_payload_v1() -> ExecutionPayloadV1 {
        ExecutionPayloadV1 {
            parent_hash: Default::default(),
            fee_recipient: Default::default(),
            state_root: Default::default(),
            receipts_root: Default::default(),
            logs_bloom: Default::default(),
            prev_randao: Default::default(),
            block_number: 0,
            gas_limit: 0,
            gas_used: 0,
            timestamp: 0,
            extra_data: Default::default(),
            base_fee_per_gas: Default::default(),
            block_hash: Default::default(),
            transactions: vec![],
        }
    }

    #[test]
    fn test_matching_payloads() -> Result<()> {
        let mut bytes = [0u8; 1024];
        generators::rng().fill(bytes.as_mut_slice());
        let mut unstructured = Unstructured::new(&bytes);

        let parent_hash = B256::arbitrary(&mut unstructured)?;
        let transactions = Vec::<Bytes>::arbitrary(&mut unstructured)?;
        let prev_randao = B256::arbitrary(&mut unstructured)?;
        let timestamp = u64::arbitrary(&mut unstructured)?;

        let mut attributes = ScrollPayloadAttributes::arbitrary(&mut unstructured)?;
        attributes.transactions = Some(transactions.clone());
        attributes.payload_attributes.timestamp = timestamp;
        attributes.payload_attributes.prev_randao = prev_randao;

        let payload = ExecutionPayload::V1(ExecutionPayloadV1 {
            prev_randao,
            timestamp,
            transactions,
            parent_hash,
            ..default_execution_payload_v1()
        });

        assert!(matching_payloads(&attributes, &payload, parent_hash));

        Ok(())
    }

    #[test]
    fn test_mismatched_payloads() -> Result<()> {
        let mut bytes = [0u8; 1024];
        generators::rng().fill(bytes.as_mut_slice());
        let mut unstructured = Unstructured::new(&bytes);

        let parent_hash = B256::arbitrary(&mut unstructured)?;
        let transactions = Vec::<Bytes>::arbitrary(&mut unstructured)?;
        let prev_randao = B256::arbitrary(&mut unstructured)?;
        let timestamp = u64::arbitrary(&mut unstructured)?;

        let attributes = ScrollPayloadAttributes::arbitrary(&mut unstructured)?;
        let payload = ExecutionPayload::V1(ExecutionPayloadV1 {
            prev_randao,
            timestamp,
            transactions,
            parent_hash,
            ..default_execution_payload_v1()
        });

        assert!(!matching_payloads(&attributes, &payload, parent_hash));

        Ok(())
    }
}