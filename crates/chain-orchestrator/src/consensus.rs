use alloy_primitives::{Address, Signature};
use metrics::Counter;
use metrics_derive::Metrics;
use reth_primitives_traits::GotExpected;
use reth_scroll_primitives::ScrollBlock;
use rollup_node_primitives::{sig_encode_hash, ConsensusUpdate};
use scroll_network::ConsensusError;
use std::fmt::Debug;

/// A trait for consensus implementations.
pub trait Consensus: Send + Sync + Debug {
    /// Updates the current config for the consensus.
    fn update_config(&mut self, update: &ConsensusUpdate);
    /// Validates a new block with the given signature.
    fn validate_new_block(
        &self,
        block: &ScrollBlock,
        signature: &Signature,
    ) -> Result<(), ConsensusError>;
    /// Returns a boolean indicating whether the sequencer should sequence a block.
    fn should_sequence_block(&self, sequencer: &Address) -> bool;
}

/// A no-op consensus instance.
#[non_exhaustive]
#[derive(Debug, Default)]
pub struct NoopConsensus;

impl Consensus for NoopConsensus {
    fn update_config(&mut self, _: &ConsensusUpdate) {}

    fn validate_new_block(
        &self,
        _block: &ScrollBlock,
        _signature: &Signature,
    ) -> Result<(), ConsensusError> {
        Ok(())
    }

    fn should_sequence_block(&self, _sequencer: &Address) -> bool {
        true
    }
}

/// The metrics for the [`SystemContractConsensus`].
#[derive(Metrics, Clone)]
#[metrics(scope = "consensus")]
pub(crate) struct SystemContractConsensusMetrics {
    /// System contract validate new block failed counter.
    pub validate_new_block_failed: Counter,
}

/// The system contract consensus.
#[derive(Debug)]
pub struct SystemContractConsensus {
    authorized_signer: Address,

    /// The metrics for the [`SystemContractConsensus`].
    metrics: SystemContractConsensusMetrics,
}

impl SystemContractConsensus {
    /// Creates a new [`SystemContractConsensus`] consensus instance with the given authorized
    /// signers.
    pub fn new(authorized_signer: Address) -> Self {
        tracing::info!(
            target: "scroll::consensus",
            "Initialized system contract consensus with authorized signer: {authorized_signer}"
        );
        Self { authorized_signer, metrics: SystemContractConsensusMetrics::default() }
    }
}

impl Consensus for SystemContractConsensus {
    fn update_config(&mut self, update: &ConsensusUpdate) {
        match update {
            ConsensusUpdate::AuthorizedSigner(signer) => {
                tracing::info!(
                    target: "scroll::consensus",
                    "Authorized signer updated to: {signer}"
                );
                self.authorized_signer = *signer
            }
        };
    }

    fn validate_new_block(
        &self,
        block: &ScrollBlock,
        signature: &Signature,
    ) -> Result<(), ConsensusError> {
        let hash = sig_encode_hash(&block.header);
        let signer = reth_primitives_traits::crypto::secp256k1::recover_signer(signature, hash)?;

        if self.authorized_signer != signer {
            self.metrics.validate_new_block_failed.increment(1);
            return Err(ConsensusError::IncorrectSigner(GotExpected {
                got: signer,
                expected: self.authorized_signer,
            }))
        }
        Ok(())
    }

    fn should_sequence_block(&self, sequencer: &Address) -> bool {
        sequencer == &self.authorized_signer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{Signed, TxEip1559};
    use alloy_primitives::{address, b256, bloom, bytes, TxKind, B64, U256};
    use reth_primitives_traits::Header;
    use reth_scroll_primitives::ScrollBlockBody;
    use std::{str::FromStr, sync::OnceLock};

    #[test]
    fn test_should_validate_block() {
        let consensus =
            SystemContractConsensus::new(address!("d83c4892bb5aa241b63d8c4c134920111e142a20"));
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
                transactions: vec![
                    Signed::new_unhashed(TxEip1559 {
                        chain_id: 534352,
                        nonce: 145014,
                        gas_limit: 600000,
                        max_fee_per_gas: 52355852,
                        max_priority_fee_per_gas: 0,
                        to: TxKind::Call(address!("802b65b5d9016621e66003aed0b16615093f328b")),
                        value: U256::ZERO,
                        access_list: Default::default(),
                        input: bytes!("a00597a00000000000000000000000000000000000000000000000000000000001826cbe0000000000000000000000000000000000005eb6831c1aa0faf2055c7d53270e00000000000000000000000006efdbff2a14a7c8e15944d1f4a48f9f95f663a40000000000000000000000000000000000000000000000000000000000000001000000000000000000000000813df550a32d4a9d42010d057386429ad2328ed9000000000000000000000000000000000000000000000000000000006807befd"),
                    }, Signature::new(U256::from_str("12217337930795921874768983252881296563512928283585900928219483692173266513447").unwrap(), U256::from_str("37490897792770890087946325233571758133021734266092518377537449521790010698782").unwrap(), true)).into()],
                ommers: vec![],
                withdrawals: None,
            },
        };
        consensus.validate_new_block(&block, &signature).unwrap()
    }
}
