use alloy_eips::eip4844::Blob;
use alloy_primitives::Bytes;
use scroll_codec::CommitDataSource;

/// Holds the data for the codec.
pub(crate) struct CodecDataSource<'a> {
    pub(crate) calldata: &'a Bytes,
    pub(crate) blob: Option<&'a Blob>,
}

impl<'a> CommitDataSource for CodecDataSource<'a> {
    fn calldata(&self) -> &Bytes {
        self.calldata
    }

    fn blob(&self) -> Option<&Blob> {
        self.blob
    }
}
