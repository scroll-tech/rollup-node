use std::sync::OnceLock;

use alloy_primitives::{B256, bytes::BufMut, keccak256};

/// The batch header for V7.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BatchHeaderV7 {
    /// The batch version.
    pub version: u8,
    /// The index of the batch.
    pub batch_index: u64,
    /// The blob versioned hash for the batch.
    pub blob_versioned_hash: B256,
    /// The parent batch hash.
    pub parent_batch_hash: B256,
    /// The hash of the header.
    hash: OnceLock<B256>,
}

impl BatchHeaderV7 {
    pub const BYTES_LENGTH: usize = 73;

    /// Returns a new instance [`BatchHeader`].
    pub fn new(
        version: u8,
        batch_index: u64,
        blob_versioned_hash: B256,
        parent_batch_hash: B256,
    ) -> Self {
        Self { version, batch_index, blob_versioned_hash, parent_batch_hash, hash: OnceLock::new() }
    }

    /// Returns the hash of the batch header, computing it if it is queried for the first time.
    pub fn hash(&self) -> &B256 {
        self.hash.get_or_init(|| self.hash_slow())
    }

    /// Computes the hash for the header.
    fn hash_slow(&self) -> B256 {
        let mut bytes = Vec::<u8>::with_capacity(Self::BYTES_LENGTH);
        bytes.put_slice(&self.version.to_be_bytes());
        bytes.put_slice(&self.batch_index.to_be_bytes());
        bytes.put_slice(&self.blob_versioned_hash.0);
        bytes.put_slice(&self.parent_batch_hash.0);

        keccak256(bytes)
    }
}

#[cfg(test)]
mod tests {
    use crate::decoding::v7::BatchHeaderV7;

    use alloy_primitives::b256;

    #[test]
    fn test_should_hash_header() {
        // <https://sepolia.etherscan.io/tx/0x6dca39d9f34790c4d6a86e84638cee69681a84e8d95d684e9a85d0a629ed26c5>
        let header = BatchHeaderV7::new(
            7,
            86131,
            b256!("0133cdebc827838f8c5f869b35be2b323b6bab0632e1c3b8b8201f39452ce36a"),
            b256!("0320cd98cb921dbb1ddc0ef9a578d5e07dee23ba0483d90fb2ea274b745c343c"),
        );

        let expected = b256!("c0976bb0928a08f7792cbf54b9ed142b97a4bafd0248014491eb29ae2b0ade12");
        assert_eq!(header.hash_slow(), expected);
    }
}
