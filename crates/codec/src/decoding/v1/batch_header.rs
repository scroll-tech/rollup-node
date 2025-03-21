use crate::{from_be_bytes_slice_and_advance_buf, from_slice_and_advance_buf};
use std::sync::OnceLock;

use alloy_primitives::{
    B256, U256,
    bytes::{Buf, BufMut},
    keccak256,
};

/// The batch header for V1.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BatchHeaderV1 {
    /// The batch version.
    version: u8,
    /// The index of the batch.
    batch_index: u64,
    /// Number of L1 messages popped in the batch.
    l1_message_popped: u64,
    /// Number of total L1 messages popped after the batch.
    total_l1_message_popped: u64,
    /// The data hash of the batch.
    data_hash: B256,
    /// The blob versioned hash for the batch.
    blob_versioned_hash: B256,
    /// The parent batch hash.
    parent_batch_hash: B256,
    /// A bitmap to indicate which L1 messages are skipped in the batch.
    skipped_l1_message_bitmap: Vec<U256>,
    /// The hash of the header.
    hash: OnceLock<B256>,
}

impl BatchHeaderV1 {
    pub const BYTES_LENGTH: usize = 121;

    /// Returns a new instance [`BatchHeader`].
    pub fn new(
        version: u8,
        batch_index: u64,
        l1_message_popped: u64,
        total_l1_message_popped: u64,
        data_hash: B256,
        blob_versioned_hash: B256,
        parent_batch_hash: B256,
        skipped_l1_message_bitmap: Vec<U256>,
    ) -> Self {
        Self {
            version,
            batch_index,
            l1_message_popped,
            total_l1_message_popped,
            data_hash,
            blob_versioned_hash,
            parent_batch_hash,
            skipped_l1_message_bitmap,
            hash: OnceLock::new(),
        }
    }

    /// Tries to read from the input buffer into the [`BatchHeader`].
    /// Returns [`None`] if the buffer.len() < [`BatchHeader::BYTES_LENGTH`].
    pub fn try_from_buf(buf: &mut &[u8]) -> Option<Self> {
        if buf.len() < Self::BYTES_LENGTH {
            return None
        }

        let version = from_be_bytes_slice_and_advance_buf!(u8, buf);
        let batch_index = from_be_bytes_slice_and_advance_buf!(u64, buf);

        let l1_message_popped = from_be_bytes_slice_and_advance_buf!(u64, buf);
        let total_l1_message_popped = from_be_bytes_slice_and_advance_buf!(u64, buf);

        let data_hash = from_slice_and_advance_buf!(B256, buf);
        let blob_versioned_hash = from_slice_and_advance_buf!(B256, buf);
        let parent_batch_hash = from_slice_and_advance_buf!(B256, buf);

        let skipped_l1_message_bitmap: Vec<_> =
            buf.chunks(32).map(|chunk| U256::from_be_slice(chunk)).collect();

        // check leftover bytes are correct.
        if buf.len() as u64 != (l1_message_popped + 255) / 256 * 32 {
            return None
        }
        buf.advance(skipped_l1_message_bitmap.len() * 32);

        Some(Self {
            version,
            batch_index,
            l1_message_popped,
            total_l1_message_popped,
            data_hash,
            blob_versioned_hash,
            parent_batch_hash,
            skipped_l1_message_bitmap,
            hash: OnceLock::new(),
        })
    }

    /// Returns the hash of the batch header, computing it if it is queried for the first time.
    pub fn hash(&self) -> &B256 {
        self.hash.get_or_init(|| self.hash_slow())
    }

    /// Computes the hash for the header.
    fn hash_slow(&self) -> B256 {
        let mut bytes = Vec::<u8>::with_capacity(
            Self::BYTES_LENGTH + self.skipped_l1_message_bitmap.len() * 32,
        );
        bytes.put_slice(&self.version.to_be_bytes());
        bytes.put_slice(&self.batch_index.to_be_bytes());
        bytes.put_slice(&self.l1_message_popped.to_be_bytes());
        bytes.put_slice(&self.total_l1_message_popped.to_be_bytes());
        bytes.put_slice(&self.data_hash.0);
        bytes.put_slice(&self.blob_versioned_hash.0);
        bytes.put_slice(&self.parent_batch_hash.0);

        let skipped_l1_message_flat_bitmap = self
            .skipped_l1_message_bitmap
            .iter()
            .flat_map(|u| u.to_be_bytes::<32>())
            .collect::<Vec<_>>();
        bytes.put_slice(&skipped_l1_message_flat_bitmap);

        keccak256(bytes)
    }
}

#[cfg(test)]
mod tests {
    use crate::decoding::{test_utils::read_to_bytes, v1::BatchHeaderV1};

    use alloy_primitives::b256;
    use alloy_sol_types::SolCall;
    use scroll_l1::abi::calls::commitBatchCall;

    #[test]
    fn test_should_decode_header() -> eyre::Result<()> {
        // <https://etherscan.io/tx/0x27d73eef6f0de411f8db966f0def9f28c312a0ae5cfb1ac09ec23f8fa18b005b>
        let raw_commit_calldata = read_to_bytes("./src/testdata/calldata_v1.bin")?;
        let commit_calldata = commitBatchCall::abi_decode(&raw_commit_calldata, true)?;

        let mut raw_batch_header = &*commit_calldata.parentBatchHeader.to_vec();
        let header = BatchHeaderV1::try_from_buf(&mut raw_batch_header).unwrap();

        let expected = BatchHeaderV1::new(
            1,
            206594,
            0,
            815396,
            b256!("e58ee8f9c15196600f9e618806bc835d1fdc35fe2467ed71adcf1a7c47d4e7eb"),
            b256!("014edb613b68d298710004d463b92eed58aab3af7386b8f32127af53b33fc9be"),
            b256!("a1aece1f54b8a429b121d61619b49f5c9da3b83d924c21c418160531e1319658"),
            vec![],
        );
        assert_eq!(header, expected);

        Ok(())
    }

    #[test]
    fn test_should_hash_header() {
        // <https://etherscan.io/tx/0x27d73eef6f0de411f8db966f0def9f28c312a0ae5cfb1ac09ec23f8fa18b005b>
        let header = BatchHeaderV1::new(
            1,
            206594,
            0,
            815396,
            b256!("e58ee8f9c15196600f9e618806bc835d1fdc35fe2467ed71adcf1a7c47d4e7eb"),
            b256!("014edb613b68d298710004d463b92eed58aab3af7386b8f32127af53b33fc9be"),
            b256!("a1aece1f54b8a429b121d61619b49f5c9da3b83d924c21c418160531e1319658"),
            vec![],
        );

        let expected = b256!("46B1C269784F1FC6CC12A04A65496D2D2B24D77EF582E893A510A963D09F4661");
        assert_eq!(header.hash_slow(), expected);
    }
}
