//! ZSTD helpers.

use crate::error::DecodingError;

use alloy_eips::eip4844::USABLE_BYTES_PER_BLOB;
use alloy_primitives::bytes::Buf;
use zstd_safe::{DCtx, InBuffer, OutBuffer, get_error_name};

/// The ZSTD magic number for zstd compressed data header.
const ZSTD_MAGIC_NUMBER: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];

/// Uncompress the provided data.
pub fn decompress_blob_data(data: &[u8]) -> Result<Vec<u8>, DecodingError> {
    let mut header_data = ZSTD_MAGIC_NUMBER.to_vec();
    header_data.extend_from_slice(data);
    // a capacity of twice the useful bytes per blob is a rough estimation of the amount of decoded
    // data.
    let mut output = Vec::with_capacity(2 * USABLE_BYTES_PER_BLOB);

    let mut ctx = DCtx::create();
    ctx.init().map_err(get_error_name).expect("context init erro");
    ctx.load_dictionary(&[]).map_err(get_error_name).expect("dictionary load error");

    let buf_size = DCtx::in_size();
    let mut out_buffer = vec![0u8; 8192];
    let src = &mut &*header_data;

    while let Ok(size) = decompress_loop(&mut ctx, src, &mut out_buffer) {
        // break in case we aren't decoding anything anymore.
        if size == 0 {
            break
        }
        output.extend_from_slice(&out_buffer[..size]);
    }

    Ok(output)
}

fn init_decompression(ctx: &mut DCtx, buf: &mut [u8]) -> Result<(), DecodingError> {
    // taken from <https://github.com/gyscos/zstd-rs/blob/main/src/stream/zio/reader.rs#L119>
    let mut output_buf = OutBuffer::around(buf);
    let mut input_buf = InBuffer::around(b"");
    ctx.decompress_stream(&mut output_buf, &mut input_buf)
        .map_err(|err| DecodingError::DecompressionFailed(get_error_name(err)))
        .map(|_| ())
}

fn decompress_loop(
    ctx: &mut DCtx,
    src: &mut &[u8],
    dst: &mut [u8],
) -> Result<usize, DecodingError> {
    init_decompression(ctx, dst)?;
    let mut input_buffer = InBuffer::around(src);
    let mut output_buffer = OutBuffer::around(dst);

    let _ = ctx
        .decompress_stream(&mut output_buffer, &mut input_buffer)
        .map_err(|err| DecodingError::DecompressionFailed(get_error_name(err)))?;

    let (read, written) = (input_buffer.pos(), output_buffer.pos());
    dbg!(read, written);
    src.advance(read);

    Ok(written)
}
