/// Errors related to Scroll primitives.
#[derive(Debug)]
pub enum RollupNodePrimitiveError {
    /// Error decoding an execution payload.
    ExecutionPayloadDecodeError(alloy_eips::eip2718::Eip2718Error),
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
        }
    }
}
