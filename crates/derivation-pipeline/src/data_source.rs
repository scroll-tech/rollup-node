use alloy_primitives::Bytes;
use scroll_codec::CommitDataSource;

/// Holds the data for the codec.
pub(crate) struct CodecDataSource<Calldata, Blob> {
    pub(crate) calldata: Calldata,
    pub(crate) blob: Option<Blob>,
}

impl<Calldata: AsRef<Bytes>, Blob: AsRef<alloy_eips::eip4844::Blob>> CommitDataSource
    for CodecDataSource<Calldata, Blob>
{
    fn calldata(&self) -> &Bytes {
        self.calldata.as_ref()
    }

    fn blob(&self) -> Option<&alloy_eips::eip4844::Blob> {
        self.blob.as_ref().map(|b| b.as_ref())
    }
}
