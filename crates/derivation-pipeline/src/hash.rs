use alloy_primitives::{bytes::BufMut, keccak256, B256};
use rollup_node_primitives::L1MessageWithBlockNumber;
use scroll_codec::{BlockContext, L2Block};

/// Computes the data hash for the batch.
pub fn compute_data_hash(
    version: u8,
    chunks: Vec<&[L2Block]>,
    l1_messages: Vec<&[L1MessageWithBlockNumber]>,
) -> B256 {
    let mut chunk_hashes = Vec::with_capacity(chunks.len() * 32);
    for (blocks, messages) in chunks.into_iter().zip(l1_messages.into_iter()) {
        chunk_hashes.append(&mut compute_chunk_data_hash(version, blocks, messages).to_vec());
    }

    keccak256(chunk_hashes)
}

/// Compute the hash for the chunk.
fn compute_chunk_data_hash(
    version: u8,
    l2_blocks: &[L2Block],
    l1_messages: &[L1MessageWithBlockNumber],
) -> B256 {
    let mut capacity = l2_blocks.len() * (BlockContext::BYTES_LENGTH - 2) + l1_messages.len() * 32;
    if version == 0 {
        capacity += l2_blocks.iter().map(|b| b.transactions.len()).sum::<usize>();
    }
    let mut buf = Vec::with_capacity(capacity);

    for block in l2_blocks {
        let context = block.context.to_be_bytes();
        // we don't use the last 2 bytes.
        buf.put_slice(&context[..BlockContext::BYTES_LENGTH - 2]);
    }

    for l1_message in l1_messages {
        buf.put_slice(l1_message.transaction.tx_hash().as_slice())
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
    use super::compute_data_hash;
    use alloy_primitives::b256;

    use scroll_codec::decoding::{test_utils::read_to_bytes, v0::decode_v0, v1::decode_v1};

    #[test]
    fn test_should_compute_data_hash_v0() -> eyre::Result<()> {
        // <https://etherscan.io/tx/0x2c7bb77d6086befd9bdcf936479fd246d1065cbd2c6aff55b1d39a67aff965c1>
        let raw_calldata = read_to_bytes("../codec/testdata/calldata_v0.bin")?;
        let blocks_count = [1, 1, 1, 1, 3, 1, 1, 1, 1, 1, 1, 3, 6, 4, 2];
        let mut blocks = decode_v0(&raw_calldata)?.into_iter();
        let chunks = blocks_count
            .into_iter()
            .map(|i| (&mut blocks).take(i).collect::<Vec<_>>())
            .collect::<Vec<_>>();

        let hash = compute_data_hash(
            0,
            chunks.iter().map(|b| b.as_slice()).collect(),
            vec![&[]; blocks_count.len()],
        );

        assert_eq!(hash, b256!("33e608dbf683c1ee03a34d01de52f67d60a0563b7e713b65a7395bb3b646f71f"));

        Ok(())
    }

    #[test]
    fn test_should_compute_data_hash_v1() -> eyre::Result<()> {
        // <https://etherscan.io/tx/0x27d73eef6f0de411f8db966f0def9f28c312a0ae5cfb1ac09ec23f8fa18b005b>
        let raw_calldata = read_to_bytes("../codec/testdata/calldata_v1.bin")?;
        let blob = read_to_bytes("../codec/testdata/blob_v1.bin")?;
        let mut blocks = decode_v1(&raw_calldata, &blob)?.into_iter();
        let blocks_count = [1usize, 1, 1, 1, 2, 1, 1, 1, 1, 1, 1];
        let chunks = blocks_count
            .into_iter()
            .map(|i| (&mut blocks).take(i).collect::<Vec<_>>())
            .collect::<Vec<_>>();

        let hash = compute_data_hash(
            1,
            chunks.iter().map(|b| b.as_slice()).collect(),
            vec![&[]; blocks_count.len()],
        );

        assert_eq!(hash, b256!("c20f5914a772663080f8a77955b33814a04f7a19c880536e562a1bcfd5343a37"));

        Ok(())
    }
}
