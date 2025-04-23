use alloy_chains::NamedChain;
use alloy_consensus::Header;
use alloy_primitives::{
    address, keccak256,
    private::{alloy_rlp, alloy_rlp::Encodable},
    Address, Signature, B256, U256,
};
use alloy_provider::Provider;
use reth_scroll_primitives::ScrollBlock;
use scroll_network::ConsensusError;

/// The address of the system contract on Sepolia.
const SEPOLIA_SYSTEM_CONTRAT_ADDRESS: Address =
    address!("C706Ba9fa4fedF4507CB7A898b4766c1bbf9be57");

/// The address of the system contract on Mainnet.
const MAINNET_SYSTEM_CONTRAT_ADDRESS: Address =
    address!("8432728A257646449245558B8b7Dbe51A16c7a4D");

/// The storage slot of the authorized signer.
const AUTHORIZED_SIGNER_STORAGE_SLOT: U256 = U256::from_limbs([0x67, 0x0, 0x0, 0x0]);

/// A trait for consensus implementations.
pub trait Consensus {
    /// Validates a new block with the given signature.
    fn validate_new_block(
        &self,
        block: &ScrollBlock,
        signature: &Signature,
    ) -> Result<(), ConsensusError>;
}

/// A Proof of Authority consensus instance.
#[derive(Debug)]
pub struct PoAConsensus {
    authorized_signers: Vec<Address>,
}

impl PoAConsensus {
    /// Creates a new [`PoAConsensus`] consensus instance with the given authorized signers.
    pub const fn new(authorized_signers: Vec<Address>) -> Self {
        Self { authorized_signers }
    }

    /// Initialize the [`PoAConsensus`] by fetching the authorized signers from L1 system contract.
    pub async fn initialize<P: Provider>(&mut self, provider: P, chain: NamedChain) {
        let system_contract_address = match chain {
            NamedChain::Scroll => MAINNET_SYSTEM_CONTRAT_ADDRESS,
            NamedChain::ScrollSepolia => SEPOLIA_SYSTEM_CONTRAT_ADDRESS,
            _ => panic!("unsupported chain"),
        };
        let authorized_signer = provider
            .get_storage_at(system_contract_address, AUTHORIZED_SIGNER_STORAGE_SLOT)
            .await
            .expect("failed to fetch PoAConsensus authorized signer");

        let authorized_signer = Address::from_slice(&authorized_signer.to_be_bytes::<32>()[12..]);
        self.authorized_signers.push(authorized_signer);
    }
}

impl Consensus for PoAConsensus {
    fn validate_new_block(
        &self,
        block: &ScrollBlock,
        signature: &Signature,
    ) -> Result<(), ConsensusError> {
        let hash = sig_encode_hash(&block.header);
        let signer = reth_primitives_traits::crypto::secp256k1::recover_signer(signature, hash)
            .map_err(|_| ConsensusError::Signature)?;

        if !self.authorized_signers.contains(&signer) {
            return Err(ConsensusError::Signature)
        }
        Ok(())
    }
}

/// Encode and hash the header for signature. The function is similar to `Header::encode` but skips
/// the `extra_data` field.
fn sig_encode_hash(header: &Header) -> B256 {
    let out = &mut Vec::new();
    let list_header =
        alloy_rlp::Header { list: true, payload_length: sig_header_payload_length(header) };
    list_header.encode(out);
    header.parent_hash.encode(out);
    header.ommers_hash.encode(out);
    header.beneficiary.encode(out);
    header.state_root.encode(out);
    header.transactions_root.encode(out);
    header.receipts_root.encode(out);
    header.logs_bloom.encode(out);
    header.difficulty.encode(out);
    U256::from(header.number).encode(out);
    U256::from(header.gas_limit).encode(out);
    U256::from(header.gas_used).encode(out);
    header.timestamp.encode(out);
    header.mix_hash.encode(out);
    header.nonce.encode(out);

    // Encode all the fork specific fields
    if let Some(ref base_fee) = header.base_fee_per_gas {
        U256::from(*base_fee).encode(out);
    }
    keccak256(&out)
}

/// Returns the header payload length for signature.
fn sig_header_payload_length(header: &Header) -> usize {
    let mut length = 0;
    length += header.parent_hash.length();
    length += header.ommers_hash.length();
    length += header.beneficiary.length();
    length += header.state_root.length();
    length += header.transactions_root.length();
    length += header.receipts_root.length();
    length += header.logs_bloom.length();
    length += header.difficulty.length();
    length += U256::from(header.number).length();
    length += U256::from(header.gas_limit).length();
    length += U256::from(header.gas_used).length();
    length += header.timestamp.length();
    length += header.mix_hash.length();
    length += header.nonce.length();

    if let Some(base_fee) = header.base_fee_per_gas {
        // Adding base fee length if it exists.
        length += U256::from(base_fee).length();
    }
    length
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::TxEip1559;
    use alloy_primitives::{address, b256, bloom, bytes, TxKind, B64, U256};
    use reth_primitives_traits::Header;
    use reth_scroll_primitives::{ScrollBlockBody, ScrollTransactionSigned};
    use scroll_alloy_consensus::ScrollTypedTransaction;
    use std::{str::FromStr, sync::OnceLock};

    #[test]
    fn test_should_validate_block() {
        let consensus =
            PoAConsensus::new(vec![address!("d83c4892bb5aa241b63d8c4c134920111e142a20")]);
        let signature = Signature::from_raw(&bytes!("6d2b8ef87f0956ea4dd10fb0725fa7196ad80c6d567a161f6b4367f95b5de6ec279142b540d3b248f08ed337bb962fa3fd83d21de622f7d6c8207272558fd15a00")).unwrap();

        let tx_hash = OnceLock::new();
        tx_hash.get_or_init(|| {
            b256!("f257edab88796a76f6d19a9fadad44b2b16c28e7aa70322cc4c6abc857128998")
        });

        let block = ScrollBlock {
            header: Header {
                parent_hash: b256!("3ccf36621e1f75cd1bfd2ac39ff6a00d8a5bec02e52aa7064a4860a0d02d6013"),
                ommers_hash: b256!("1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347"),
                beneficiary: address!("0000000000000000000000000000000000000000"),
                state_root: b256!("bc6c2ccfdb3e0e78b53134f583e6d42760adcaebb23e7a6bab59503c4b98daeb"),
                transactions_root: b256!("a11e1b74f0aada603d9b4e645a57d60259dc2545c0372b88e16e6b59cecac8b6"),
                receipts_root: b256!("72de16699164034d2ed9930a021820e32e103ea7162b6f6a9a535d0a98f3fac0"),
                logs_bloom: bloom!("0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000008000400000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001000000000008000000000010000040000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
                difficulty: U256::ONE,
                number: 14916920,
                gas_limit: 10000000,
                gas_used: 245760,
                timestamp: 1745337938,
                extra_data: bytes!("0x"),
                mix_hash: b256!("0000000000000000000000000000000000000000000000000000000000000000"),
                nonce: B64::from(0x0000000000000000i64),
                base_fee_per_gas: Some(40306624),
                withdrawals_root: None,
                blob_gas_used: None,
                excess_blob_gas: None,
                parent_beacon_block_root: None,
                requests_hash: None,
            },
            body: ScrollBlockBody {
                transactions: vec![ScrollTransactionSigned {
                    hash: tx_hash,
                    signature:
                    Signature::new(U256::from_str("12217337930795921874768983252881296563512928283585900928219483692173266513447").unwrap(), U256::from_str("37490897792770890087946325233571758133021734266092518377537449521790010698782").unwrap(), true) ,
                    transaction: ScrollTypedTransaction::Eip1559(TxEip1559{
                        chain_id: 534352,
                        nonce: 145014,
                        gas_limit: 600000,
                        max_fee_per_gas: 52355852,
                        max_priority_fee_per_gas: 0,
                        to: TxKind::Call(address!("802b65b5d9016621e66003aed0b16615093f328b")),
                        value: U256::ZERO,
                        access_list: Default::default(),
                        input: bytes!("a00597a00000000000000000000000000000000000000000000000000000000001826cbe0000000000000000000000000000000000005eb6831c1aa0faf2055c7d53270e00000000000000000000000006efdbff2a14a7c8e15944d1f4a48f9f95f663a40000000000000000000000000000000000000000000000000000000000000001000000000000000000000000813df550a32d4a9d42010d057386429ad2328ed9000000000000000000000000000000000000000000000000000000006807befd"),
                    }) ,
                }],
                ommers: vec![],
                withdrawals: None,
            },
        };

        consensus.validate_new_block(&block, &signature).unwrap()
    }
}
