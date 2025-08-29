//! Decoding implementations for the commit data.

/// Batch related structures.
pub mod batch;

/// Batch header related structures.
pub mod batch_header;

/// Blob related helpers.
pub mod blob;

/// Constants involved in the decoding process.
pub mod constants;

mod macros;

/// V0 implementation of the decoding.
pub mod v0;

/// V1 implementation of the decoding.
pub mod v1;

/// V2 implementation of the decoding.
pub mod v2;

/// V3 implementation of the decoding.
pub mod v3;

/// V4 implementation of the decoding.
pub mod v4;

/// V7 implementation of the decoding.
pub mod v7;

/// Decoded payload.
pub(crate) mod payload;

/// Tests utils.
#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

/// Decoding implementation for a transaction.
pub mod transaction;
