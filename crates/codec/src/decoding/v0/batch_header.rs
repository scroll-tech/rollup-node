use crate::{
    decoding::constants::SKIPPED_L1_MESSAGE_BITMAP_ITEM_BYTES_SIZE, error::DecodingError,
    from_be_bytes_slice_and_advance_buf, from_slice_and_advance_buf,
};

use alloy_primitives::{
    bytes::{Buf, BufMut},
    keccak256, B256,
};

/// The batch header for V0.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BatchHeaderV0 {
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
    /// The parent batch hash.
    pub parent_batch_hash: B256,
    /// A bitmap to indicate which L1 messages are skipped in the batch.
    pub skipped_l1_message_bitmap: Vec<u8>,
}

impl BatchHeaderV0 {
    pub const BYTES_LENGTH: usize = 89;

    /// Returns a new instance [`BatchHeaderV0`].
    pub fn new(
        version: u8,
        batch_index: u64,
        l1_message_popped: u64,
        total_l1_message_popped: u64,
        data_hash: B256,
        parent_batch_hash: B256,
        skipped_l1_message_bitmap: Vec<u8>,
    ) -> Self {
        Self {
            version,
            batch_index,
            l1_message_popped,
            total_l1_message_popped,
            data_hash,
            parent_batch_hash,
            skipped_l1_message_bitmap,
        }
    }

    /// Tries to read from the input buffer into the [`BatchHeaderV0`].
    /// Returns [`DecodingError::Eof`] if the buffer.len() < [`BatchHeaderV0::BYTES_LENGTH`].
    pub fn try_from_buf(buf: &mut &[u8]) -> Result<Self, DecodingError> {
        if buf.len() < Self::BYTES_LENGTH {
            return Err(DecodingError::Eof)
        }

        let version = from_be_bytes_slice_and_advance_buf!(u8, buf);
        let batch_index = from_be_bytes_slice_and_advance_buf!(u64, buf);

        let l1_message_popped = from_be_bytes_slice_and_advance_buf!(u64, buf);
        let total_l1_message_popped = from_be_bytes_slice_and_advance_buf!(u64, buf);

        let data_hash = from_slice_and_advance_buf!(B256, buf);
        let parent_batch_hash = from_slice_and_advance_buf!(B256, buf);

        let skipped_l1_message_bitmap: Vec<_> = buf
            .chunks(SKIPPED_L1_MESSAGE_BITMAP_ITEM_BYTES_SIZE)
            .flatten()
            .rev()
            .copied()
            .collect();

        // check leftover bytes are correct.
        if buf.len() as u64 !=
            l1_message_popped.div_ceil(256) * SKIPPED_L1_MESSAGE_BITMAP_ITEM_BYTES_SIZE as u64
        {
            return Err(DecodingError::Eof)
        }
        buf.advance(skipped_l1_message_bitmap.len());

        Ok(Self {
            version,
            batch_index,
            l1_message_popped,
            total_l1_message_popped,
            data_hash,
            parent_batch_hash,
            skipped_l1_message_bitmap,
        })
    }

    /// Computes the hash for the header.
    pub fn hash_slow(&self) -> B256 {
        let mut bytes = Vec::<u8>::with_capacity(
            Self::BYTES_LENGTH +
                self.skipped_l1_message_bitmap.len() * SKIPPED_L1_MESSAGE_BITMAP_ITEM_BYTES_SIZE,
        );
        bytes.put_slice(&self.version.to_be_bytes());
        bytes.put_slice(&self.batch_index.to_be_bytes());
        bytes.put_slice(&self.l1_message_popped.to_be_bytes());
        bytes.put_slice(&self.total_l1_message_popped.to_be_bytes());
        bytes.put_slice(&self.data_hash.0);
        bytes.put_slice(&self.parent_batch_hash.0);

        bytes.put_slice(&self.skipped_l1_message_bitmap);

        keccak256(bytes)
    }
}

#[cfg(test)]
mod tests {
    use crate::decoding::{test_utils::read_to_bytes, v0::BatchHeaderV0};

    use alloy_primitives::b256;
    use alloy_sol_types::SolCall;
    use scroll_l1::abi::calls::commitBatchCall;

    #[test]
    fn test_should_decode_header() -> eyre::Result<()> {
        // <https://etherscan.io/tx/0x2c7bb77d6086befd9bdcf936479fd246d1065cbd2c6aff55b1d39a67aff965c1>
        let raw_commit_calldata = read_to_bytes("./testdata/calldata_v0.bin")?;
        let commit_calldata = commitBatchCall::abi_decode(&raw_commit_calldata)?;

        let mut raw_batch_header = &*commit_calldata.parent_batch_header.to_vec();
        let header = BatchHeaderV0::try_from_buf(&mut raw_batch_header)?;

        let expected = BatchHeaderV0::new(
            0,
            9,
            1,
            33,
            b256!("2aa3eeb5adebb96a49736583c744b89b0b3be45056e8e178106a42ab2cd1a063"),
            b256!("c0173d7e3561501cf57913763c7c34716216092a222a99fe8b85dcb466730f56"),
            vec![0; 32],
        );
        assert_eq!(header, expected);

        Ok(())
    }

    #[test]
    fn test_should_decode_header_with_skipped_l1_messages() -> eyre::Result<()> {
        // <https://sepolia.etherscan.io/tx/0xacacfe48bed1944d6586ca8f0bec3ecd10ea0a99e104517f75845b8602dcab31>
        let raw_commit_calldata =
            read_to_bytes("./testdata/calldata_v0_with_skipped_l1_messages.bin")?;
        let commit_calldata = commitBatchCall::abi_decode(&raw_commit_calldata)?;

        let mut raw_batch_header = &*commit_calldata.parent_batch_header.to_vec();
        let header = BatchHeaderV0::try_from_buf(&mut raw_batch_header)?;

        let expected = BatchHeaderV0::new(
            0,
            100,
            3,
            22,
            b256!("4867e8b3c751abf5f0f8cd8e3e91f78ff15011b48b981ad742cb42dfd746844c"),
            b256!("b4d0a673c704d567eebcd758802ce87cf103b16acbae7c52b2807928fd8dc76e"),
            vec![
                1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0,
            ],
        );
        assert_eq!(header, expected);

        Ok(())
    }

    #[test]
    fn test_should_hash_header() {
        // <https://etherscan.io/tx/0x2c7bb77d6086befd9bdcf936479fd246d1065cbd2c6aff55b1d39a67aff965c1>
        let header = BatchHeaderV0::new(
            0,
            9,
            1,
            33,
            b256!("2aa3eeb5adebb96a49736583c744b89b0b3be45056e8e178106a42ab2cd1a063"),
            b256!("c0173d7e3561501cf57913763c7c34716216092a222a99fe8b85dcb466730f56"),
            vec![0; 32],
        );

        let expected = b256!("A7F7C528E1827D3E64E406C76DE6C750D5FC3DE3DE4386E6C69958A89461D064");
        assert_eq!(header.hash_slow(), expected);
    }
}
