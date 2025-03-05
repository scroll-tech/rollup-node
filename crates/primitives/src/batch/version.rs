use crate::BatchError;

/// The version of the batch input data.
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub enum BatchInputVersion {
    /// The first version of the batch input data.
    V1 = 1,
    /// The second version of the batch input data.
    V2 = 2,
}

impl TryFrom<u8> for BatchInputVersion {
    type Error = BatchError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(BatchInputVersion::V1),
            2 => Ok(BatchInputVersion::V2),
            _ => Err(BatchError::InvalidBatchVersion(value)),
        }
    }
}
