use crate::{
    decoding::{v0::BatchHeaderV0, v1::BatchHeaderV1, v3::BatchHeaderV3, v7::BatchHeaderV7},
    DecodingError,
};

/// The batch header.
pub enum BatchHeader {
    /// The batch header for V0.
    V0(BatchHeaderV0),
    /// The batch header for V1.
    V1(BatchHeaderV1),
    /// The batch header for V3.
    V3(BatchHeaderV3),
    /// The batch header for V7.
    V7(BatchHeaderV7),
}

impl BatchHeader {
    /// Returns the total number of L1 messages popped after the batch, if the version supports it.
    pub fn total_l1_message_popped(&self) -> Option<u64> {
        match self {
            BatchHeader::V0(header) => Some(header.total_l1_message_popped),
            BatchHeader::V1(header) => Some(header.total_l1_message_popped),
            BatchHeader::V3(header) => Some(header.total_l1_message_popped),
            BatchHeader::V7(_) => None,
        }
    }

    /// Tries to read from the input buffer into the appropriate batch header version.
    /// Returns [`DecodingError::Eof`] if the buffer is empty or does not contain enough bytes for
    /// the specific version.
    pub fn try_from_buf(buf: &mut &[u8]) -> Result<Self, DecodingError> {
        if buf.is_empty() {
            return Err(DecodingError::Eof);
        }
        let version = buf[0];

        match version {
            0 => Ok(BatchHeader::V0(BatchHeaderV0::try_from_buf(buf)?)),
            1..=2 => Ok(BatchHeader::V1(BatchHeaderV1::try_from_buf(buf)?)),
            3.. => Ok(BatchHeader::V3(BatchHeaderV3::try_from_buf(buf)?)),
        }
    }
}
