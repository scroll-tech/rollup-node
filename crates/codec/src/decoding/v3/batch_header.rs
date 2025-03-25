use crate::{from_be_bytes_slice_and_advance_buf, from_slice_and_advance_buf};

use alloy_primitives::{B256, bytes::BufMut, keccak256};

/// The batch header for V3.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BatchHeaderV3 {
    /// The batch version.
    pub version: u8,
    /// The index of the batch.
    pub batch_index: u64,
    /// Number of L1 messages popped in the batch.
    pub l1_message_popped: u64,
    /// Number of total L1 messages popped after the batch.
    pub total_l1_message_popped: u64,
    /// The data hash of the batch.
    pub data_hash: B256,
    /// The blob versioned hash for the batch.
    pub blob_versioned_hash: B256,
    /// The parent batch hash.
    pub parent_batch_hash: B256,
    /// The timestamp of the last block in the batch.
    pub last_block_timestamp: u64,
    /// The blob data proof: z (32 bytes) and y (32 bytes).
    pub blob_data_proof: [B256; 2],
}

impl BatchHeaderV3 {
    pub const BYTES_LENGTH: usize = 193;

    /// Returns a new instance [`BatchHeaderV3`].
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        version: u8,
        batch_index: u64,
        l1_message_popped: u64,
        total_l1_message_popped: u64,
        data_hash: B256,
        blob_versioned_hash: B256,
        parent_batch_hash: B256,
        last_block_timestamp: u64,
        blob_data_proof: [B256; 2],
    ) -> Self {
        Self {
            version,
            batch_index,
            l1_message_popped,
            total_l1_message_popped,
            data_hash,
            blob_versioned_hash,
            parent_batch_hash,
            last_block_timestamp,
            blob_data_proof,
        }
    }

    /// Tries to read from the input buffer into the [`BatchHeaderV3`].
    /// Returns [`None`] if the buffer.len() < [`BatchHeaderV3::BYTES_LENGTH`].
    pub fn try_from_buf(buf: &mut &[u8]) -> Option<Self> {
        if buf.len() < Self::BYTES_LENGTH {
            return None;
        }

        let version = from_be_bytes_slice_and_advance_buf!(u8, buf);
        let batch_index = from_be_bytes_slice_and_advance_buf!(u64, buf);

        let l1_message_popped = from_be_bytes_slice_and_advance_buf!(u64, buf);
        let total_l1_message_popped = from_be_bytes_slice_and_advance_buf!(u64, buf);

        let data_hash = from_slice_and_advance_buf!(B256, buf);
        let blob_versioned_hash = from_slice_and_advance_buf!(B256, buf);
        let parent_batch_hash = from_slice_and_advance_buf!(B256, buf);

        let last_block_timestamp = from_be_bytes_slice_and_advance_buf!(u64, buf);

        let blob_data_proof_z = from_slice_and_advance_buf!(B256, buf);
        let blob_data_proof_y = from_slice_and_advance_buf!(B256, buf);

        Some(Self {
            version,
            batch_index,
            l1_message_popped,
            total_l1_message_popped,
            data_hash,
            blob_versioned_hash,
            parent_batch_hash,
            last_block_timestamp,
            blob_data_proof: [blob_data_proof_z, blob_data_proof_y],
        })
    }

    /// Computes the hash for the header.
    pub fn hash_slow(&self) -> B256 {
        let mut bytes = Vec::<u8>::with_capacity(Self::BYTES_LENGTH);
        bytes.put_slice(&self.version.to_be_bytes());
        bytes.put_slice(&self.batch_index.to_be_bytes());
        bytes.put_slice(&self.l1_message_popped.to_be_bytes());
        bytes.put_slice(&self.total_l1_message_popped.to_be_bytes());
        bytes.put_slice(&self.data_hash.0);
        bytes.put_slice(&self.blob_versioned_hash.0);
        bytes.put_slice(&self.parent_batch_hash.0);
        bytes.put_slice(&self.last_block_timestamp.to_be_bytes());
        bytes.put_slice(&self.blob_data_proof[0].0);
        bytes.put_slice(&self.blob_data_proof[1].0);

        keccak256(bytes)
    }
}

#[cfg(test)]
mod tests {
    use crate::decoding::{test_utils::read_to_bytes, v3::BatchHeaderV3};

    use alloy_primitives::b256;
    use alloy_sol_types::SolCall;
    use scroll_l1::abi::calls::commitBatchWithBlobProofCall;

    #[test]
    fn test_should_decode_header() -> eyre::Result<()> {
        // <https://etherscan.io/tx/0xee0afe29207fe23626387bc8eb209ab751c1fee9c18e3d6ec7a5edbcb5a4fed4>
        let raw_commit_calldata = read_to_bytes("./testdata/calldata_v4_compressed.bin")?;
        let commit_calldata = commitBatchWithBlobProofCall::abi_decode(&raw_commit_calldata, true)?;

        let mut raw_batch_header = &*commit_calldata.parent_batch_header.to_vec();
        let header = BatchHeaderV3::try_from_buf(&mut raw_batch_header).unwrap();

        let expected = BatchHeaderV3::new(
            4,
            314188,
            0,
            932910,
            b256!("c12a808c7097fd258c6fbaf7522274b0f872a347c3ad57b92ec08132a2372535"),
            b256!("01f71b48cb81e381f052c23f0036b4625d0cc9bc003ea821f5a143d4b838faa8"),
            b256!("6529a8718e42ee479ecddb356875ac3fcbfc52ea5df1f0489666200e886fc46d"),
            1725454956,
            [
                b256!("0bd522bcaf28f037b8f59e1df03bb3dba8981efa98dfd4875ed8eefc7bbd1127"),
                b256!("64f0b033e86cecf5e5047b15930b44920c7de1b558ab78f6ee5927e582b47a8b"),
            ],
        );
        assert_eq!(header, expected);

        Ok(())
    }

    #[test]
    fn test_should_hash_header() {
        // <https://etherscan.io/tx/0xee0afe29207fe23626387bc8eb209ab751c1fee9c18e3d6ec7a5edbcb5a4fed4>
        let header = BatchHeaderV3::new(
            4,
            314188,
            0,
            932910,
            b256!("c12a808c7097fd258c6fbaf7522274b0f872a347c3ad57b92ec08132a2372535"),
            b256!("01f71b48cb81e381f052c23f0036b4625d0cc9bc003ea821f5a143d4b838faa8"),
            b256!("6529a8718e42ee479ecddb356875ac3fcbfc52ea5df1f0489666200e886fc46d"),
            1725454956,
            [
                b256!("0bd522bcaf28f037b8f59e1df03bb3dba8981efa98dfd4875ed8eefc7bbd1127"),
                b256!("64f0b033e86cecf5e5047b15930b44920c7de1b558ab78f6ee5927e582b47a8b"),
            ],
        );

        let expected = b256!("E5E11C9BDAECA56D8741D3BFBE0EC087D09DC0BD8D6C660485F8D6E4A7D5560A");
        assert_eq!(header.hash_slow(), expected);
    }
}
