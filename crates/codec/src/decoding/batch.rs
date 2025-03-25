use crate::{BlockContext, L2Block, decoding::payload::PayloadData};
use alloy_primitives::{B256, bytes::BufMut, keccak256};
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

    /// Computes the hash for the batch, using the provided L1 messages associated with each block.
    pub fn try_hash(&self, l1_messages: &[TxL1Message]) -> Option<B256> {
        // From version 7 and above, the batch doesn't have a data hash.
        if self.version >= 7 {
            return None;
        }

        let chunks_count = self.chunks_block_count.as_ref()?;
        let blocks_buf = &mut (&**self.data.l2_blocks());
        let l1_messages = &mut (&*l1_messages);

        let mut chunk_hashes = Vec::with_capacity(chunks_count.len() * 32);

        for chunk_count in chunks_count {
            // slice the blocks at chunk_count and filter l1 message.
            let blocks = blocks_buf.get(..*chunk_count)?;
            let l1_messages_count = blocks.iter().map(|b| b.context.num_l1_messages as usize).sum();
            let messages = l1_messages.get(..l1_messages_count)?;

            // compute the chunk data hash.
            chunk_hashes.append(&mut hash_chunk(self.version, blocks, messages).to_vec());

            // advance the buffer.
            *blocks_buf = blocks_buf.get(*chunk_count..).unwrap_or(&[]);
        }

        Some(keccak256(chunk_hashes))
    }
}

/// Compute the hash for the chunk.
fn hash_chunk(version: u8, l2_blocks: &[L2Block], l1_messages: &[TxL1Message]) -> B256 {
    // reserve the correct capacity.
    let mut capacity = l2_blocks.len() * (BlockContext::BYTES_LENGTH - 2) + l1_messages.len() * 32;
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

    for l1_message in l1_messages {
        buf.put_slice(l1_message.tx_hash().as_slice())
    }

    // for v0, we add the l2 transaction hashes.
    if version == 0 {
        for block in l2_blocks {
            for tx in &block.transactions {
                buf.put_slice(keccak256(&tx.0).as_slice());
            }
        }
    }

    keccak256(buf)
}

#[cfg(test)]
mod tests {
    use crate::decoding::{test_utils::read_to_bytes, v0::decode_v0};

    use crate::decoding::v1::decode_v1;
    use alloy_primitives::b256;

    #[test]
    fn test_should_compute_data_hash_v0() -> eyre::Result<()> {
        // <https://etherscan.io/tx/0x2c7bb77d6086befd9bdcf936479fd246d1065cbd2c6aff55b1d39a67aff965c1>
        let raw_calldata = read_to_bytes("../codec/testdata/calldata_v0.bin")?;
        let batch = decode_v0(&raw_calldata)?;

        let hash = batch.try_hash(&[]).unwrap();

        assert_eq!(hash, b256!("33e608dbf683c1ee03a34d01de52f67d60a0563b7e713b65a7395bb3b646f71f"));

        Ok(())
    }

    #[test]
    fn test_should_compute_data_hash_v1() -> eyre::Result<()> {
        // <https://etherscan.io/tx/0x27d73eef6f0de411f8db966f0def9f28c312a0ae5cfb1ac09ec23f8fa18b005b>
        let raw_calldata = read_to_bytes("../codec/testdata/calldata_v1.bin")?;
        let blob = read_to_bytes("../codec/testdata/blob_v1.bin")?;
        let batch = decode_v1(&raw_calldata, &blob)?;

        let hash = batch.try_hash(&[]).unwrap();

        assert_eq!(hash, b256!("c20f5914a772663080f8a77955b33814a04f7a19c880536e562a1bcfd5343a37"));

        Ok(())
    }
}
