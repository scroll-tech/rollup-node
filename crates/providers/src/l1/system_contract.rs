use crate::L1ProviderError;

use alloy_primitives::{Address, U256};
use alloy_provider::Provider;

/// The storage slot of the authorized signer.
pub const AUTHORIZED_SIGNER_STORAGE_SLOT: U256 = U256::from_limbs([0x67, 0x0, 0x0, 0x0]);

/// Provides access to the L1 system contract.
#[async_trait::async_trait]
pub trait SystemContractProvider {
    /// Returns the authorized signer from the system contract on the L1.
    async fn authorized_signer(&self, address: Address) -> Result<Address, L1ProviderError>;
}

#[async_trait::async_trait]
impl<P: Provider> SystemContractProvider for P {
    async fn authorized_signer(&self, address: Address) -> Result<Address, L1ProviderError> {
        let signer = self.get_storage_at(address, AUTHORIZED_SIGNER_STORAGE_SLOT).await?;
        Ok(Address::from_slice(&signer.to_be_bytes::<32>()[12..]))
    }
}
