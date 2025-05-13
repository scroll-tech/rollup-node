use crate::L1ProviderError;

use alloy_chains::NamedChain;
use alloy_primitives::{address, Address, U256};
use alloy_provider::Provider;

/// The storage slot of the authorized signer.
pub const AUTHORIZED_SIGNER_STORAGE_SLOT: U256 = U256::from_limbs([0x67, 0x0, 0x0, 0x0]);

/// The address of the system contract on Sepolia.
pub const SEPOLIA_SYSTEM_CONTRAT_ADDRESS: Address =
    address!("C706Ba9fa4fedF4507CB7A898b4766c1bbf9be57");

/// The address of the system contract on Mainnet.
pub const MAINNET_SYSTEM_CONTRAT_ADDRESS: Address =
    address!("8432728A257646449245558B8b7Dbe51A16c7a4D");

/// Provides access to the L1 system contract.
#[async_trait::async_trait]
pub trait SystemContractProvider {
    /// Returns the authorized signer from the system contract on the L1 corresponding to the
    /// provided chain id.
    async fn authorized_signer(&self) -> Result<Address, L1ProviderError>;
}

#[async_trait::async_trait]
impl<P: Provider> SystemContractProvider for P {
    async fn authorized_signer(&self) -> Result<Address, L1ProviderError> {
        let chain_id = self.get_chain_id().await?;
        let named_chain: NamedChain =
            chain_id.try_into().map_err(|_| L1ProviderError::Other("unexpected chain id"))?;
        let address = match named_chain {
            NamedChain::Mainnet => MAINNET_SYSTEM_CONTRAT_ADDRESS,
            NamedChain::Sepolia => SEPOLIA_SYSTEM_CONTRAT_ADDRESS,
            _ => return Err(L1ProviderError::Other("expected Mainnet or Sepolia chain")),
        };

        let signer = self.get_storage_at(address, AUTHORIZED_SIGNER_STORAGE_SLOT).await?;
        Ok(Address::from_slice(&signer.to_be_bytes::<32>()[12..]))
    }
}
