use super::*;

/// A type that represents the result of the engine driver future.
#[derive(Debug)]
pub(crate) enum EngineDriverFutureResult {
    BlockImport(
        Result<
            (Option<BlockInfo>, Option<BlockImportOutcome>, PayloadStatusEnum),
            EngineDriverError,
        >,
    ),
    L1Consolidation(Result<ConsolidationOutcome, EngineDriverError>),
    PayloadBuildingJob(Result<ScrollBlock, EngineDriverError>),
}

impl
    From<
        Result<
            (Option<BlockInfo>, Option<BlockImportOutcome>, PayloadStatusEnum),
            EngineDriverError,
        >,
    > for EngineDriverFutureResult
{
    fn from(
        value: Result<
            (Option<BlockInfo>, Option<BlockImportOutcome>, PayloadStatusEnum),
            EngineDriverError,
        >,
    ) -> Self {
        Self::BlockImport(value)
    }
}

impl From<Result<ConsolidationOutcome, EngineDriverError>> for EngineDriverFutureResult {
    fn from(value: Result<ConsolidationOutcome, EngineDriverError>) -> Self {
        Self::L1Consolidation(value)
    }
}

impl From<Result<ScrollBlock, EngineDriverError>> for EngineDriverFutureResult {
    fn from(value: Result<ScrollBlock, EngineDriverError>) -> Self {
        Self::PayloadBuildingJob(value)
    }
}
