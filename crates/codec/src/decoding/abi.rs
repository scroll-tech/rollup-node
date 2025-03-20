use alloy_sol_types::sol;

sol! {
    function commitBatch(uint8 _version,bytes _parentBatchHeader,bytes[] _chunks,bytes _skippedL1MessageBitmap) external;
    function commitBatchWithBlobProof(uint8 _version,bytes _parentBatchHeader,bytes[] _chunks,bytes _skippedL1MessageBitmap,bytes _blobDataProof);
}
