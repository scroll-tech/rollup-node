//! A stateless derivation pipeline for Scroll.
//!
//! This crate provides a simple implementation of a derivation pipeline that transforms a batch
//! into payload attributes for block building.

#![cfg_attr(not(feature = "std"), no_std)]

mod data_source;
mod error;

#[cfg(not(feature = "std"))]
extern crate alloc as std;

use crate::{data_source::CodecDataSource, error::DerivationPipelineError};
use std::vec::Vec;

use alloy_primitives::B256;
use alloy_rpc_types_engine::PayloadAttributes;
use reth_scroll_chainspec::SCROLL_FEE_VAULT_ADDRESS;
use rollup_node_primitives::BatchCommitData;
use rollup_node_providers::L1Provider;
use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;
use scroll_codec::Codec;

/// Returns an iterator over [`ScrollPayloadAttributes`] from the [`BatchCommitData`] and a
/// [`L1Provider`].
pub fn derive<P: L1Provider>(
    batch: BatchCommitData,
    l1_provider: &mut P,
) -> Result<impl Iterator<Item = ScrollPayloadAttributes> + use<'_, P>, DerivationPipelineError> {
    // fetch the blob then decode the input batch.
    let blob = batch.blob_versioned_hash.and_then(|hash| l1_provider.blob(hash));
    let data = CodecDataSource { calldata: batch.calldata.as_ref(), blob: blob.as_ref() };
    let decoded = Codec::decode(&data)?;

    // set the cursor for the l1 provider.
    let data = &decoded.data;
    if let Some(index) = data.queue_index_start() {
        l1_provider.set_index_cursor(index)
    } else if let Some(hash) = data.prev_l1_message_queue_hash() {
        l1_provider.set_hash_cursor(*hash);
        // we skip the first l1 message, as we are interested in the one starting after
        // prev_l1_message_queue_hash.
        let _ = l1_provider.next_l1_message();
    } else {
        return Err(DerivationPipelineError::MissingL1MessageQueueCursor)
    }

    let iter = decoded.data.into_l2_blocks().into_iter().map(|mut block| {
        // query the appropriate amount of l1 messages.
        let mut txs = (0..block.context.num_l1_messages)
            .map(|_| l1_provider.next_l1_message())
            .map(|tx| {
                let mut bytes = Vec::new();
                tx.eip2718_encode(&mut bytes);
                bytes.into()
            })
            .collect::<Vec<_>>();

        // add the block transactions.
        txs.append(&mut block.transactions);

        // construct the payload attributes.
        ScrollPayloadAttributes {
            payload_attributes: PayloadAttributes {
                timestamp: block.context.timestamp,
                prev_randao: B256::ZERO,
                // TODO: this should be based off the current configuration value.
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_eips::eip4844::Blob;
    use alloy_primitives::{address, b256, bytes, U256};
    use core::cell::RefCell;
    use rollup_node_providers::L1MessageProvider;
    use scroll_alloy_consensus::TxL1Message;
    use scroll_codec::decoding::test_utils::read_to_bytes;
    use std::sync::Arc;

    struct TestL1MessageProvider {
        messages: RefCell<Vec<TxL1Message>>,
    }

    impl L1BlobProvider for TestL1MessageProvider {
        fn blob(&self, _hash: B256) -> Option<Blob> {
            None
        }
    }

    impl L1MessageProvider for TestL1MessageProvider {
        fn next_l1_message(&self) -> TxL1Message {
            self.messages.borrow_mut().remove(0)
        }

        fn set_index_cursor(&mut self, _index: u64) {}

        fn set_hash_cursor(&mut self, _hash: B256) {}
    }

    #[test]
    fn test_should_derive_batch() -> eyre::Result<()> {
        // https://etherscan.io/tx/0x8f4f0fcab656aa81589db5b53255094606c4624bfd99702b56b2debaf6211f48
        let raw_calldata = read_to_bytes("./testdata/calldata_v0.bin")?;
        let batch_data = BatchCommitData {
            hash: b256!("7f26edf8e3decbc1620b4d2ba5f010a6bdd10d6bb16430c4f458134e36ab3961"),
            index: 12,
            block_number: 18319648,
            calldata: Arc::new(raw_calldata),
            blob_versioned_hash: None,
        };

        let l1_messages = vec![TxL1Message {
            queue_index: 33,
            gas_limit: 168000,
            to: address!("781e90f1c8Fc4611c9b7497C3B47F99Ef6969CbC"),
            value: U256::ZERO,
            sender: address!("7885BcBd5CeCEf1336b5300fb5186A12DDD8c478"),
            input: bytes!("8ef1332e0000000000000000000000007f2b8c31f88b6006c382775eea88297ec1e3e9050000000000000000000000006ea73e05adc79974b931123675ea8f78ffdacdf0000000000000000000000000000000000000000000000000006a94d74f430000000000000000000000000000000000000000000000000000000000000000002100000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000a4232e8748000000000000000000000000ca266224613396a0e8d4c2497dbc4f33dd6cdeff000000000000000000000000ca266224613396a0e8d4c2497dbc4f33dd6cdeff000000000000000000000000000000000000000000000000006a94d74f4300000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
        },TxL1Message {
            queue_index: 34,
            gas_limit: 168000,
            to: address!("781e90f1c8fc4611c9b7497c3b47f99ef6969cbc"),
            value: U256::ZERO,
            sender: address!("7885BcBd5CeCEf1336b5300fb5186A12DDD8c478"),
            input: bytes!("8ef1332e0000000000000000000000007f2b8c31f88b6006c382775eea88297ec1e3e9050000000000000000000000006ea73e05adc79974b931123675ea8f78ffdacdf000000000000000000000000000000000000000000000000000470de4df820000000000000000000000000000000000000000000000000000000000000000002200000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000a4232e8748000000000000000000000000982fe4a7cbd74bb3422ebe46333c3e8046c12c7f000000000000000000000000982fe4a7cbd74bb3422ebe46333c3e8046c12c7f00000000000000000000000000000000000000000000000000470de4df8200000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
        }];
        let mut provider = TestL1MessageProvider { messages: RefCell::new(l1_messages) };

        let mut attributes = derive(batch_data, &mut provider)?;
        let attribute = attributes.find(|a| a.payload_attributes.timestamp == 1696935384).unwrap();

        let expected = ScrollPayloadAttributes{
            payload_attributes: PayloadAttributes{
                timestamp: 1696935384,
                prev_randao: B256::ZERO,
                suggested_fee_recipient: SCROLL_FEE_VAULT_ADDRESS,
                withdrawals: None,
                parent_beacon_block_root: None,
            },
            transactions: Some(vec![bytes!("7ef901b7218302904094781e90f1c8fc4611c9b7497c3b47f99ef6969cbc80b901848ef1332e0000000000000000000000007f2b8c31f88b6006c382775eea88297ec1e3e9050000000000000000000000006ea73e05adc79974b931123675ea8f78ffdacdf0000000000000000000000000000000000000000000000000006a94d74f430000000000000000000000000000000000000000000000000000000000000000002100000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000a4232e8748000000000000000000000000ca266224613396a0e8d4c2497dbc4f33dd6cdeff000000000000000000000000ca266224613396a0e8d4c2497dbc4f33dd6cdeff000000000000000000000000000000000000000000000000006a94d74f4300000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000947885bcbd5cecef1336b5300fb5186a12ddd8c478"), bytes!("7ef901b7228302904094781e90f1c8fc4611c9b7497c3b47f99ef6969cbc80b901848ef1332e0000000000000000000000007f2b8c31f88b6006c382775eea88297ec1e3e9050000000000000000000000006ea73e05adc79974b931123675ea8f78ffdacdf000000000000000000000000000000000000000000000000000470de4df820000000000000000000000000000000000000000000000000000000000000000002200000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000a4232e8748000000000000000000000000982fe4a7cbd74bb3422ebe46333c3e8046c12c7f000000000000000000000000982fe4a7cbd74bb3422ebe46333c3e8046c12c7f00000000000000000000000000000000000000000000000000470de4df8200000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000947885bcbd5cecef1336b5300fb5186a12ddd8c478")]),
            no_tx_pool: true,
        };
        assert_eq!(attribute, expected);

        let attribute = attributes.last().unwrap();
        let expected = ScrollPayloadAttributes{
            payload_attributes: PayloadAttributes{
                timestamp: 1696935657,
                prev_randao: B256::ZERO,
                suggested_fee_recipient: SCROLL_FEE_VAULT_ADDRESS,
                withdrawals: None,
                parent_beacon_block_root: None,
            },
            transactions: Some(vec![bytes!("f88c8202658417d7840082a4f294530000000000000000000000000000000000000280a4bede39b500000000000000000000000000000000000000000000000000000001669aa2f583104ec4a07461e6555f927393ebdf5f183738450c3842bc3b86a1db7549d9bee21fadd0b1a06d7ba96897bd9fb8e838a327d3ca34be66da11955f10d1fb2264949071e9e8cd")]),
            no_tx_pool: true,
        };
        assert_eq!(attribute, expected);

        Ok(())
    }
}
