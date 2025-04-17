use super::*;

/// A type that represents the result of the engine driver future.
pub(crate) enum EngineDriverFutureResult {
    BlockImport(Result<(Option<BlockInfo>, Option<BlockImportOutcome>), EngineDriverError>),
    L1Consolidation(Result<(BlockInfo, bool), EngineDriverError>),
    PayloadBuildingJob(Result<ScrollBlock, EngineDriverError>),
}

impl From<Result<(Option<BlockInfo>, Option<BlockImportOutcome>), EngineDriverError>>
    for EngineDriverFutureResult
{
    fn from(
        value: Result<(Option<BlockInfo>, Option<BlockImportOutcome>), EngineDriverError>,
    ) -> Self {
        Self::BlockImport(value)
    }
}

impl From<Result<(BlockInfo, bool), EngineDriverError>> for EngineDriverFutureResult {
    fn from(value: Result<(BlockInfo, bool), EngineDriverError>) -> Self {
        Self::L1Consolidation(value)
    }
}

impl From<Result<ScrollBlock, EngineDriverError>> for EngineDriverFutureResult {
    fn from(value: Result<ScrollBlock, EngineDriverError>) -> Self {
        Self::PayloadBuildingJob(value)
    }
}
