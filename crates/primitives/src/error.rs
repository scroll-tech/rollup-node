use std::string::String;

/// Errors related to Scroll primitives.
#[derive(Debug)]
pub enum RollupNodePrimitiveError {
    /// Error decoding an execution payload.
    ExecutionPayloadDecodeError(alloy_eips::eip2718::Eip2718Error),
    /// Error parsing a rollup node primitive.
    ParsingError(RollupNodePrimitiveParsingError),
}

impl From<alloy_eips::eip2718::Eip2718Error> for RollupNodePrimitiveError {
    fn from(err: alloy_eips::eip2718::Eip2718Error) -> Self {
        Self::ExecutionPayloadDecodeError(err)
    }
}

impl core::fmt::Display for RollupNodePrimitiveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ExecutionPayloadDecodeError(e) => {
                write!(f, "execution payload decode error: {e}")
            }
            Self::ParsingError(e) => {
                write!(f, "parsing error: {e:?}")
            }
        }
    }
}

/// An error that occurs when parsing a batch status from a string.
#[derive(Debug)]
pub enum RollupNodePrimitiveParsingError {
    /// Error parsing batch status from string.
    InvalidBatchStatusString(String),
}
