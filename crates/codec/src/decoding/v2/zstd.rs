//! ZSTD helpers.

use std::io::Read;

use alloy_eips::eip4844::USABLE_BYTES_PER_BLOB;
use zstd::Decoder;

/// The ZSTD magic number for zstd compressed data header.
const ZSTD_MAGIC_NUMBER: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];

/// Uncompress the provided data.
pub fn decompress_blob_data(data: &[u8]) -> Vec<u8> {
    let mut header_data = ZSTD_MAGIC_NUMBER.to_vec();
    header_data.extend_from_slice(data);
    let mut output = Vec::with_capacity(USABLE_BYTES_PER_BLOB);
    let mut decompressor = Decoder::new(header_data.as_slice()).unwrap();
    let mut dst = vec![0u8; 8192];
    while let Ok(size) = decompressor.read(&mut dst) {
        if size == 0 {
            break
        }
        output.extend_from_slice(&dst[..size]);
    }

    output
}
