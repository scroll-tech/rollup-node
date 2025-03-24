use alloy_primitives::Bytes;

/// Read the file provided at `path` as a [`Bytes`].
pub fn read_to_bytes<P: AsRef<std::path::Path>>(path: P) -> eyre::Result<Bytes> {
    use std::str::FromStr;
    Ok(Bytes::from_str(&std::fs::read_to_string(path)?)?)
}
