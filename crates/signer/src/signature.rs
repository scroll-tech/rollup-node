use alloy_primitives::Signature;

/// Trait for custom signature byte representation
pub trait SignatureAsBytes {
    /// Custom implementation of as_bytes 
    /// We remove the `+ 27` offset from the v value
    /// and use the original v value directly.
    fn sig_as_bytes(&self) -> [u8; 65];
}

impl SignatureAsBytes for Signature {
    /// Custom implementation of as_bytes 
    /// We remove the `+ 27` offset from the v value
    /// and use the original v value directly.
    #[inline]
    fn sig_as_bytes(&self) -> [u8; 65] {
        let mut sig = [0u8; 65];
        sig[..32].copy_from_slice(&self.r().to_be_bytes::<32>());
        sig[32..64].copy_from_slice(&self.s().to_be_bytes::<32>());
        sig[64] = self.v() as u8;
        sig
    }
}