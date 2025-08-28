//! Custom signature implementation that wraps alloy_primitives::Signature
//! and provides a custom implementation for the as_bytes method.

use alloy_primitives::{B256, U256};
use std::ops::Deref;

/// A custom signature wrapper that provides a custom as_bytes implementation
/// while delegating all other functionality to the underlying alloy_primitives::Signature.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct Signature {
    inner: alloy_primitives::Signature,
}

impl Signature {
    /// Creates a new Signature from r, s, and y_parity values.
    #[inline]
    pub const fn new(r: U256, s: U256, y_parity: bool) -> Self {
        Self {
            inner: alloy_primitives::Signature::new(r, s, y_parity),
        }
    }

    /// Custom implementation of as_bytes 
    /// We remove the `+ 27` offset from the v value
    /// and use the original v value directly.
    #[inline]
    pub fn as_bytes(&self) -> [u8; 65] {
        let mut sig = [0u8; 65];
        sig[..32].copy_from_slice(&self.inner.r().to_be_bytes::<32>());
        sig[32..64].copy_from_slice(&self.inner.s().to_be_bytes::<32>());
        sig[64] = self.inner.v() as u8;
        sig
    }

    /// Creates a signature from scalars and parity.
    #[inline]
    pub fn from_scalars_and_parity(r: B256, s: B256, parity: bool) -> Self {
        Self {
            inner: alloy_primitives::Signature::from_scalars_and_parity(r, s, parity),
        }
    }

    /// Parses a 65-byte long raw signature.
    #[inline]
    pub fn from_raw(bytes: &[u8]) -> Result<Self, alloy_primitives::SignatureError> {
        Ok(Self {
            inner: alloy_primitives::Signature::from_raw(bytes)?,
        })
    }

    /// Parses a 65-byte long raw signature.
    #[inline]
    pub fn from_raw_array(bytes: &[u8; 65]) -> Result<Self, alloy_primitives::SignatureError> {
        Ok(Self {
            inner: alloy_primitives::Signature::from_raw_array(bytes)?,
        })
    }

    /// Parses a signature from a byte slice, with a v value.
    #[inline]
    pub fn from_bytes_and_parity(bytes: &[u8], parity: bool) -> Self {
        Self {
            inner: alloy_primitives::Signature::from_bytes_and_parity(bytes, parity),
        }
    }

    /// Returns the inner alloy_primitives::Signature for cases where you need
    /// the original functionality.
    #[inline]
    pub fn inner(&self) -> &alloy_primitives::Signature {
        &self.inner
    }

    /// Consumes this signature and returns the inner alloy_primitives::Signature.
    #[inline]
    pub fn into_inner(self) -> alloy_primitives::Signature {
        self.inner
    }
}

// Implement Deref to automatically delegate all other methods to the inner signature
impl Deref for Signature {
    type Target = alloy_primitives::Signature;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

// Implement From conversions
impl From<alloy_primitives::Signature> for Signature {
    fn from(sig: alloy_primitives::Signature) -> Self {
        Self { inner: sig }
    }
}

impl From<Signature> for alloy_primitives::Signature {
    fn from(sig: Signature) -> Self {
        sig.inner
    }
}

impl From<&Signature> for alloy_primitives::Signature {
    fn from(sig: &Signature) -> Self {
        sig.inner
    }
}

// Implement TryFrom for byte arrays
impl TryFrom<&[u8]> for Signature {
    type Error = alloy_primitives::SignatureError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        Self::from_raw(bytes)
    }
}

// Implement FromStr
impl std::str::FromStr for Signature {
    type Err = alloy_primitives::SignatureError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            inner: alloy_primitives::Signature::from_str(s)?,
        })
    }
}

// Implement Display
impl std::fmt::Display for Signature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{}", alloy_primitives::hex::encode(self.as_bytes()))
    }
}

// Implement conversions to byte arrays using our custom as_bytes method
impl From<&Signature> for [u8; 65] {
    #[inline]
    fn from(value: &Signature) -> [u8; 65] {
        value.as_bytes()
    }
}

impl From<Signature> for [u8; 65] {
    #[inline]
    fn from(value: Signature) -> [u8; 65] {
        value.as_bytes()
    }
}

impl From<&Signature> for Vec<u8> {
    #[inline]
    fn from(value: &Signature) -> Self {
        value.as_bytes().to_vec()
    }
}

impl From<Signature> for Vec<u8> {
    #[inline]
    fn from(value: Signature) -> Self {
        value.as_bytes().to_vec()
    }
}

/// A wrapper that adapts any Signer<alloy_primitives::Signature> to work with our custom Signature
#[derive(Debug)]
pub struct SignerAdapter<T> {
    inner: T,
}

impl<T> SignerAdapter<T> {
    /// Creates a new SignerAdapter wrapping the provided signer.
    pub fn new(signer: T) -> Self {
        Self { inner: signer }
    }
}

#[async_trait::async_trait]
impl<T> alloy_signer::Signer<Signature> for SignerAdapter<T>
where
    T: alloy_signer::Signer<alloy_primitives::Signature> + Send + Sync,
{
    fn address(&self) -> alloy_primitives::Address {
        self.inner.address()
    }

    fn chain_id(&self) -> Option<alloy_primitives::ChainId> {
        self.inner.chain_id()
    }

    fn set_chain_id(&mut self, chain_id: Option<alloy_primitives::ChainId>) {
        self.inner.set_chain_id(chain_id)
    }

    async fn sign_hash(&self, hash: &alloy_primitives::B256) -> Result<Signature, alloy_signer::Error> {
        let sig = self.inner.sign_hash(hash).await.map_err(|_| alloy_signer::Error::other("signing failed"))?;
        Ok(sig.into())
    }

    async fn sign_message(&self, message: &[u8]) -> Result<Signature, alloy_signer::Error> {
        let sig = self.inner.sign_message(message).await.map_err(|_| alloy_signer::Error::other("signing failed"))?;
        Ok(sig.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_custom_as_bytes() {
        let sig = Signature::new(
            U256::from_str("18515461264373351373200002665853028612451056578545711640558177340181847433846").unwrap(),
            U256::from_str("46948507304638947509940763649030358759909902576025900602547168820602576006531").unwrap(),
            false,
        );

        let expected = alloy_primitives::hex!(
            "0x28ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa63627667cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d831b"
        );
        assert_eq!(sig.as_bytes(), expected);
    }

    #[test]
    fn test_deref_functionality() {
        let sig = Signature::new(
            U256::from_str("18515461264373351373200002665853028612451056578545711640558177340181847433846").unwrap(),
            U256::from_str("46948507304638947509940763649030358759909902576025900602547168820602576006531").unwrap(),
            false,
        );

        // Test that we can access methods from the inner signature via Deref
        assert_eq!(sig.r(), sig.inner.r());
        assert_eq!(sig.s(), sig.inner.s());
        assert_eq!(sig.v(), sig.inner.v());
    }

    #[test]
    fn test_from_str() {
        let sig_str = "0x28ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa63627667cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d831b";
        let sig = Signature::from_str(sig_str).expect("could not parse signature");
        
        // Test that our custom as_bytes works correctly
        let bytes = sig.as_bytes();
        assert_eq!(bytes.len(), 65);
    }

    #[test]
    fn test_conversions() {
        let inner_sig = alloy_primitives::Signature::new(
            U256::from_str("18515461264373351373200002665853028612451056578545711640558177340181847433846").unwrap(),
            U256::from_str("46948507304638947509940763649030358759909902576025900602547168820602576006531").unwrap(),
            false,
        );

        // Test From conversions
        let sig: Signature = inner_sig.into();
        let back_to_inner: alloy_primitives::Signature = sig.into();
        
        assert_eq!(inner_sig.r(), back_to_inner.r());
        assert_eq!(inner_sig.s(), back_to_inner.s());
        assert_eq!(inner_sig.v(), back_to_inner.v());
    }
}
