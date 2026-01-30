//! ZSTD helpers.

use std::io::Read;

#[cfg(not(any(feature = "zstd", feature = "ruzstd")))]
compile_error!("You must enable exactly one of the `zstd` or `ruzstd` features");
#[cfg(all(feature = "zstd", feature = "ruzstd"))]
compile_error!("Features `zstd` and `ruzstd` are mutually exclusive");

/// The ZSTD magic number for zstd compressed data header.
const ZSTD_MAGIC_NUMBER: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];

/// Result type for Zstd operations.
type Result<T> = std::result::Result<T, ZstdError>;

/// Zstd error type.
#[derive(Debug)]
pub struct ZstdError(Box<dyn std::error::Error + Send + Sync>);

impl std::fmt::Display for ZstdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ZstdError: {}", self.0)
    }
}

impl std::error::Error for ZstdError {}

impl ZstdError {
    /// Consumes the error and returns the inner error.
    pub fn into_inner(self) -> Box<dyn std::error::Error + Send + Sync> {
        self.0
    }
}

/// Uncompress the provided data.
#[cfg(feature = "zstd")]
pub fn decompress_blob_data(data: &[u8]) -> Result<Vec<u8>> {
    use zstd::Decoder;
    let mut header_data = ZSTD_MAGIC_NUMBER.to_vec();

    header_data.extend_from_slice(data);

    // init decoder and owned output data.
    let mut decoder = Decoder::new(header_data.as_slice()).map_err(|e| ZstdError(Box::new(e)))?;
    // heuristic: use data length as the allocated output capacity.
    let mut output = Vec::with_capacity(header_data.len());

    // use an output buffer with the recommended output size.
    let buffer_size = Decoder::<'static, &[u8]>::recommended_output_size();
    let mut dst = vec![0u8; buffer_size];

    while let Ok(size) = decoder.read(&mut dst) {
        // break if we read 0 bytes.
        if size == 0 {
            break
        }
        output.extend_from_slice(&dst[..size]);
    }

    Ok(output)
}

/// Uncompress the provided data.
#[cfg(feature = "ruzstd")]
pub fn decompress_blob_data(data: &[u8]) -> Result<Vec<u8>> {
    use ruzstd::decoding::StreamingDecoder;

    let mut header_data = ZSTD_MAGIC_NUMBER.to_vec();
    header_data.extend_from_slice(data);

    // init decoder and owned output data.
    let mut decoder =
        StreamingDecoder::new(header_data.as_slice()).map_err(|e| ZstdError(Box::new(e)))?;
    // heuristic: use data length as the allocated output capacity.
    let mut output = Vec::with_capacity(header_data.len());
    decoder.read_to_end(&mut output).map_err(|e| ZstdError(Box::new(e)))?;

    Ok(output)
}
