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

    // init decoder and owned output data.
    let mut decoder = Decoder::new(header_data.as_slice()).unwrap();
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

    output
}
