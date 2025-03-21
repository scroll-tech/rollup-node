use crate::decoding::{v0::BatchHeaderV0, v1::BatchHeaderV1, v3::BatchHeaderV3};

/// A batch header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchHeader {
    /// V0.
    V0(BatchHeaderV0),
    /// V1.
    V1(BatchHeaderV1),
    /// V3.
    V3(BatchHeaderV3),
}

impl BatchHeader {
    /// Returns a new [`BatchHeader`], using the byte which contains the version in order to decide
    /// on the variant.
    pub fn try_from_buf(buf: &mut &[u8]) -> Option<Self> {
        let version = buf.first()?;
        match version {
            0 => Some(BatchHeader::V0(BatchHeaderV0::try_from_buf(buf)?)),
            1..3 => Some(BatchHeader::V1(BatchHeaderV1::try_from_buf(buf)?)),
            3.. => Some(BatchHeader::V3(BatchHeaderV3::try_from_buf(buf)?)),
        }
    }

    /// Returns the total amount L1 messages popped after the batch.
    pub fn total_l1_messages_popped(&self) -> u64 {
        match self {
            BatchHeader::V0(header) => header.total_l1_message_popped,
            BatchHeader::V1(header) => header.total_l1_message_popped,
            BatchHeader::V3(header) => header.total_l1_message_popped,
        }
    }
}
