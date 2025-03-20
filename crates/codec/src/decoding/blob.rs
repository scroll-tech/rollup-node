use core::slice::Iter;

/// An iterator over a blob. The structure implements the iterator trait and will skip a byte every
/// 32 bytes. This byte is wasted due to the use of 32 bytes per field element in the blob,
/// but every field element needing to be smaller than the BLS modulus.
#[derive(Debug, Clone)]
pub struct BlobSliceIter<'a> {
    iterator: Iter<'a, u8>,
    count: usize,
}

impl<'a> BlobSliceIter<'a> {
    /// Returns a [`BlobSliceIter`] from the provided iterator.
    pub fn from_blob_slice(blob: &'a [u8]) -> BlobSliceIter<'a> {
        Self { iterator: blob.iter(), count: 0 }
    }
}

impl<'a> Iterator for BlobSliceIter<'a> {
    type Item = &'a u8;

    fn next(&mut self) -> Option<Self::Item> {
        if self.count % 32 == 0 {
            let _ = self.iterator.next();
            self.count += 1;
        }
        self.count += 1;
        self.iterator.next()
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::decoding::test_utils::read_to_bytes;
    use alloy_primitives::bytes;

    #[test]
    fn test_should_skip_unused_blob_bytes() -> eyre::Result<()> {
        let blob = read_to_bytes("./src/testdata/blob_v1.bin")?;
        let iterator = BlobSliceIter::from_blob_slice(&blob);

        let val = iterator.take(256).copied().collect::<Vec<_>>();
        let expected = bytes!("000b00000e3a0000206d00005efd00003ee1000026ca00000a480000218f0000417c000038110000305b00000d3e00000000000000000000000000000000f88d83073c99840c6aed6a82a4e494530000000000000000000000000000000000000280a4bede39b50000000000000000000000000000000000000000000000000000000015ea5f7283104ec3a03992018538e02f45171e6826c19c046e1bd79286f07475fa9103d4319af5f0caa072d510a28ac11e020807347ab425b6fd3b1b012d6631760d5d3a9bf8ed2c22c3f88d83073c9a840c6aed6a82a4e494530000000000000000000000000000000000000280a4bede39b500000000000000000000").to_vec();

        assert_eq!(val, expected);

        Ok(())
    }
}
