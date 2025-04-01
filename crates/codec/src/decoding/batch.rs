use crate::{
    decoding::{constants::KECCAK_256_DIGEST_BYTES_SIZE, payload::PayloadData},
    BlockContext, L2Block,
};

use alloy_primitives::{bytes::BufMut, keccak256, B256};
use scroll_alloy_consensus::TxL1Message;

/// The deserialized batch data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    /// The batch version.
    pub version: u8,
    /// The amount of blocks for each chunk of the batch. Only relevant for codec versions v0 ->
    /// v6.
    pub chunks_block_count: Option<Vec<usize>>,
    /// The data for the batch.
    pub data: PayloadData,
}

impl Batch {
    /// Returns a new instance of a batch.
    pub fn new(version: u8, chunks_block_count: Option<Vec<usize>>, data: PayloadData) -> Self {
        Self { version, chunks_block_count, data }
    }

    /// Computes the data hash for the batch, using the provided L1 messages associated with each
    /// block.
    pub fn try_compute_data_hash(&self, l1_messages: &[TxL1Message]) -> Option<B256> {
        // From version 7 and above, the batch doesn't have a data hash.
        if self.version >= 7 {
            return None;
        }

        let total_l1_messages: usize =
            self.data.l2_blocks().iter().map(|b| b.context.num_l1_messages as usize).sum();
        debug_assert_eq!(total_l1_messages, l1_messages.len(), "invalid l1 messages count");

        let chunks_count = self.chunks_block_count.as_ref()?;
        let blocks_buf = &mut (&**self.data.l2_blocks());
        let l1_messages_buf = &mut (&*l1_messages);

        let mut chunk_hashes =
            Vec::with_capacity(chunks_count.len() * KECCAK_256_DIGEST_BYTES_SIZE);

        for chunk_count in chunks_count {
            // slice the blocks at chunk_count.
            let blocks = blocks_buf.get(..*chunk_count)?;

            // take the correct amount of l1 messages for each block and advance the buffer.
            let l1_messages_per_block = blocks
                .iter()
                .map(|b| {
                    let num_l1_messages = b.context.num_l1_messages as usize;
                    let block_messages = l1_messages_buf.get(..num_l1_messages).unwrap_or(&[]);
                    *l1_messages_buf = l1_messages_buf.get(num_l1_messages..).unwrap_or(&[]);
                    block_messages
                })
                .collect::<Vec<_>>();

            // compute the chunk data hash.
            chunk_hashes
                .append(&mut hash_chunk(self.version, blocks, l1_messages_per_block).to_vec());

            // advance the buffer.
            *blocks_buf = blocks_buf.get(*chunk_count..).unwrap_or(&[]);
        }

        Some(keccak256(chunk_hashes))
    }
}

/// Compute the hash for the chunk.
fn hash_chunk(
    version: u8,
    l2_blocks: &[L2Block],
    l1_messages_per_block: Vec<&[TxL1Message]>,
) -> B256 {
    // reserve the correct capacity.
    let l1_messages_count: usize =
        l1_messages_per_block.iter().map(|messages| messages.len()).sum();
    let mut capacity = l2_blocks.len() * (BlockContext::BYTES_LENGTH - 2) +
        l1_messages_count * KECCAK_256_DIGEST_BYTES_SIZE;
    if version == 0 {
        capacity += l2_blocks.iter().map(|b| b.transactions.len()).sum::<usize>();
    }
    let mut buf = Vec::with_capacity(capacity);

    for block in l2_blocks {
        let context = block.context.to_be_bytes();
        // we don't use the last 2 bytes.
        // <https://github.com/scroll-tech/da-codec/blob/main/encoding/codecv0_types.go#L175>
        buf.put_slice(&context[..BlockContext::BYTES_LENGTH - 2]);
    }

    for (block, l1_messages) in l2_blocks.iter().zip(l1_messages_per_block) {
        for l1_message in l1_messages {
            buf.put_slice(l1_message.tx_hash().as_slice())
        }

        // for v0, we add the l2 transaction hashes.
        if version == 0 {
            for tx in &block.transactions {
                buf.put_slice(keccak256(&tx.0).as_slice());
            }
        }
    }

    keccak256(buf)
}

#[cfg(test)]
mod tests {
    use crate::decoding::{test_utils::read_to_bytes, v0::decode_v0, v1::decode_v1};

    use alloy_primitives::{address, b256, bytes, U256};
    use scroll_alloy_consensus::TxL1Message;

    #[test]
    fn test_should_compute_data_hash_v0() -> eyre::Result<()> {
        // <https://etherscan.io/tx/0x2c7bb77d6086befd9bdcf936479fd246d1065cbd2c6aff55b1d39a67aff965c1>
        let raw_calldata = read_to_bytes("./testdata/calldata_v0.bin")?;
        let batch = decode_v0(&raw_calldata)?;

        let hash = batch.try_compute_data_hash(&[]).unwrap();

        assert_eq!(hash, b256!("33e608dbf683c1ee03a34d01de52f67d60a0563b7e713b65a7395bb3b646f71f"));

        Ok(())
    }

    #[test]
    fn test_should_compute_data_hash_v0_with_l1_messages() -> eyre::Result<()> {
        // <https://etherscan.io/tx/0xdc0a315b25b46f4c1085e3884c63f8ede61e984e47655f7667e5f14e3df55f82>
        let raw_calldata = read_to_bytes("./testdata/calldata_v0_with_l1_messages.bin")?;
        let batch = decode_v0(&raw_calldata)?;

        let hash = batch
            .try_compute_data_hash(&[
                TxL1Message {
                    queue_index: 39,
                    gas_limit: 180000,
                    to: address!("781e90f1c8Fc4611c9b7497C3B47F99Ef6969CbC"),
                    value: U256::ZERO,
                    sender: address!("7885BcBd5CeCEf1336b5300fb5186A12DDD8c478"),
                    input: bytes!("8ef1332e000000000000000000000000f1af3b23de0a5ca3cab7261cb0061c0d779a5c7b00000000000000000000000033b60d5dd260d453cac3782b0bdc01ce846721420000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002700000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000e48431f5c1000000000000000000000000a0b86991c6218b36c1d19d4a2e9eb0ce3606eb4800000000000000000000000006efdbff2a14a7c8e15944d1f4a48f9f95f663a4000000000000000000000000c451b0191351ce308fdfd779d73814c910fc5ecb000000000000000000000000c451b0191351ce308fdfd779d73814c910fc5ecb00000000000000000000000000000000000000000000000000000005d21dba0000000000000000000000000000000000000000000000000000000000000000c0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                },
                TxL1Message {
                    queue_index: 40,
                    gas_limit: 168000,
                    to: address!("781e90f1c8Fc4611c9b7497C3B47F99Ef6969CbC"),
                    value: U256::ZERO,
                    sender: address!("7885BcBd5CeCEf1336b5300fb5186A12DDD8c478"),
                    input: bytes!("8ef1332e0000000000000000000000007f2b8c31f88b6006c382775eea88297ec1e3e9050000000000000000000000006ea73e05adc79974b931123675ea8f78ffdacdf00000000000000000000000000000000000000000000000000011c37937e08000000000000000000000000000000000000000000000000000000000000000002800000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000a4232e8748000000000000000000000000b89db2813541287a4dd1fc6801eec30595ecdc6c000000000000000000000000b89db2813541287a4dd1fc6801eec30595ecdc6c0000000000000000000000000000000000000000000000000011c37937e080000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                },
                TxL1Message {
                    queue_index: 41,
                    gas_limit: 168000,
                    to: address!("781e90f1c8Fc4611c9b7497C3B47F99Ef6969CbC"),
                    value: U256::ZERO,
                    sender: address!("7885BcBd5CeCEf1336b5300fb5186A12DDD8c478"),
                    input: bytes!("8ef1332e0000000000000000000000007f2b8c31f88b6006c382775eea88297ec1e3e9050000000000000000000000006ea73e05adc79974b931123675ea8f78ffdacdf0000000000000000000000000000000000000000000000000002386f26fc10000000000000000000000000000000000000000000000000000000000000000002900000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000a4232e87480000000000000000000000003219c394111d45757ccb68a4fd353b4f7f9660960000000000000000000000003219c394111d45757ccb68a4fd353b4f7f966096000000000000000000000000000000000000000000000000002386f26fc100000000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                },
            ])
            .unwrap();

        assert_eq!(hash, b256!("55fd647c58461d910b5bfb4539f2177ba575c9c8d578a344558976a4375cc287"));

        Ok(())
    }

    #[test]
    fn test_should_compute_data_hash_v1() -> eyre::Result<()> {
        // <https://etherscan.io/tx/0x27d73eef6f0de411f8db966f0def9f28c312a0ae5cfb1ac09ec23f8fa18b005b>
        let raw_calldata = read_to_bytes("./testdata/calldata_v1.bin")?;
        let blob = read_to_bytes("./testdata/blob_v1.bin")?;
        let batch = decode_v1(&raw_calldata, &blob)?;

        let hash = batch.try_compute_data_hash(&[]).unwrap();

        assert_eq!(hash, b256!("c20f5914a772663080f8a77955b33814a04f7a19c880536e562a1bcfd5343a37"));

        Ok(())
    }

    #[test]
    fn test_should_compute_data_hash_v1_with_l1_messages() -> eyre::Result<()> {
        // <https://etherscan.io/tx/0x30451fc1a7ad4a87f9a2616e972d2489326bafa2a41aba8cfb664aec5f727d94>
        let raw_calldata = read_to_bytes("./testdata/calldata_v1_with_l1_messages.bin")?;
        let raw_blob = read_to_bytes("./testdata/blob_v1_with_l1_messages.bin")?;
        let batch = decode_v1(&raw_calldata, &raw_blob)?;

        let l1_messages: Vec<TxL1Message> =
            serde_json::from_str(&std::fs::read_to_string("./testdata/l1_messages_v1.json")?)?;

        let hash = batch.try_compute_data_hash(&l1_messages).unwrap();

        assert_eq!(hash, b256!("e20ac534891e7f96c3a945e2aafe0a05c7079959eccd94ad217ee0f3b29ac030"));

        Ok(())
    }
}
