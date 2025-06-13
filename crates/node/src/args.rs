use crate::constants;
use alloy_primitives::Address;
use reth_scroll_chainspec::SCROLL_FEE_VAULT_ADDRESS;
use rollup_node_sequencer::L1MessageInclusionMode;
use std::path::PathBuf;

/// A struct that represents the arguments for the rollup node.
#[derive(Debug, Clone, clap::Args)]
pub struct ScrollRollupNodeConfig {
    /// Whether the rollup node should be run in test mode.
    #[arg(long)]
    pub test: bool,
    /// Database args
    #[command(flatten)]
    pub database_args: DatabaseArgs,
    /// Engine driver args.
    #[command(flatten)]
    pub engine_driver_args: EngineDriverArgs,
    /// The beacon provider arguments.
    #[command(flatten)]
    pub beacon_provider_args: BeaconProviderArgs,
    /// The L1 provider arguments
    #[command(flatten)]
    pub l1_provider_args: L1ProviderArgs,
    /// The sequencer arguments
    #[command(flatten)]
    pub sequencer_args: SequencerArgs,
    /// The network arguments
    #[command(flatten)]
    pub network_args: NetworkArgs,
    /// The signer arguments
    #[command(flatten)]
    pub signer_args: SignerArgs,
}

impl ScrollRollupNodeConfig {
    /// Validate that either signer key file or AWS KMS key ID is provided when sequencer is enabled
    pub fn validate(&self) -> Result<(), String> {
        if !self.test && self.sequencer_args.sequencer_enabled {
            if self.signer_args.key_file.is_none() && self.signer_args.aws_kms_key_id.is_none() {
                return Err("Either signer key file or AWS KMS key ID is required when sequencer is enabled".to_string());
            }
            if self.signer_args.key_file.is_some() && self.signer_args.aws_kms_key_id.is_some() {
                return Err("Cannot specify both signer key file and AWS KMS key ID".to_string());
            }
        }
        Ok(())
    }
}

/// The database arguments.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct DatabaseArgs {
    /// Database path
    #[arg(long)]
    pub path: Option<PathBuf>,
}

/// The engine driver args.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct EngineDriverArgs {
    /// The amount of block difference between the EN and the latest block received from P2P
    /// at which the engine driver triggers optimistic sync.
    #[arg(long = "engine.en-sync-trigger", default_value_t = constants::BLOCK_GAP_TRIGGER)]
    pub en_sync_trigger: u64,
}

/// The network arguments.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct NetworkArgs {
    /// A bool to represent if new blocks should be bridged from the eth wire protocol to the
    /// scroll wire protocol.
    #[arg(long = "network.bridge", default_value_t = true)]
    pub enable_eth_scroll_wire_bridge: bool,
    /// A bool that represents if the scroll wire protocol should be enabled.
    #[arg(long = "network.scroll-wire", default_value_t = true)]
    pub enable_scroll_wire: bool,
}

/// The arguments for the L1 provider.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct L1ProviderArgs {
    /// The URL for the L1 RPC.
    #[arg(long = "l1.url", id = "l1_url", value_name = "L1_URL")]
    pub url: Option<reqwest::Url>,
    /// The compute units per second for the provider.
    #[arg(long = "l1.cups", id = "l1_compute_units_per_second", value_name = "L1_COMPUTE_UNITS_PER_SECOND", default_value_t = constants::PROVIDER_COMPUTE_UNITS_PER_SECOND)]
    pub compute_units_per_second: u64,
    /// The max amount of retries for the provider.
    #[arg(long = "l1.max-retries", id = "l1_max_retries", value_name = "L1_MAX_RETRIES", default_value_t = constants::PROVIDER_MAX_RETRIES)]
    pub max_retries: u32,
    /// The initial backoff for the provider.
    #[arg(long = "l1.initial-backoff", id = "l1_initial_backoff", value_name = "L1_INITIAL_BACKOFF", default_value_t = constants::PROVIDER_INITIAL_BACKOFF)]
    pub initial_backoff: u64,
}

/// The arguments for the Beacon provider.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct BeaconProviderArgs {
    /// The URL for the Beacon chain.
    #[arg(long = "beacon.url", id = "beacon_url", value_name = "BEACON_URL")]
    pub url: Option<reqwest::Url>,
    /// The compute units per second for the provider.
    #[arg(long = "beacon.cups", id = "beacon_compute_units_per_second", value_name = "BEACON_COMPUTE_UNITS_PER_SECOND", default_value_t = constants::PROVIDER_COMPUTE_UNITS_PER_SECOND)]
    pub compute_units_per_second: u64,
    /// The max amount of retries for the provider.
    #[arg(long = "beacon.max-retries", id = "beacon_max_retries", value_name = "BEACON_MAX_RETRIES", default_value_t = constants::PROVIDER_MAX_RETRIES)]
    pub max_retries: u32,
    /// The initial backoff for the provider.
    #[arg(long = "beacon.initial-backoff", id = "beacon_initial_backoff", value_name = "BEACON_INITIAL_BACKOFF", default_value_t = constants::PROVIDER_INITIAL_BACKOFF)]
    pub initial_backoff: u64,
}

/// The arguments for the sequencer.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct SequencerArgs {
    /// Enable the scroll block sequencer.
    #[arg(long = "sequencer.enabled", default_value_t = false)]
    pub sequencer_enabled: bool,
    /// The block time for the sequencer.
    #[arg(long = "sequencer.block-time", id = "sequencer_block_time", value_name = "SEQUENCER_BLOCK_TIME", default_value_t = constants::DEFAULT_BLOCK_TIME)]
    pub block_time: u64,
    /// The payload building duration for the sequencer (milliseconds)
    #[arg(long = "sequencer.payload-building-duration", id = "sequencer_payload_building_duration", value_name = "SEQUENCER_PAYLOAD_BUILDING_DURATION", default_value_t = constants::DEFAULT_PAYLOAD_BUILDING_DURATION)]
    pub payload_building_duration: u64,
    /// The max L1 messages per block for the sequencer.
    #[arg(long = "sequencer.max-l1-messages-per-block", id = "sequencer_max_l1_messages_per_block", value_name = "SEQUENCER_MAX_L1_MESSAGES_PER_BLOCK", default_value_t = constants::DEFAULT_MAX_L1_MESSAGES_PER_BLOCK)]
    pub max_l1_messages_per_block: u64,
    /// The fee recipient for the sequencer.
    #[arg(long = "sequencer.fee-recipient", id = "sequencer_fee_recipient", value_name = "SEQUENCER_FEE_RECIPIENT", default_value_t = SCROLL_FEE_VAULT_ADDRESS)]
    pub fee_recipient: Address,
    /// L1 message inclusion mode: "finalized" or "depth:{number}"
    /// Examples: "finalized", "depth:10", "depth:6"
    #[arg(
        long = "sequencer.l1-inclusion-mode",
        id = "sequencer_l1_inclusion_mode",
        value_name = "MODE",
        default_value = "finalized",
        help = "L1 message inclusion mode. Use 'finalized' for finalized messages only, or 'depth:{number}' for block depth confirmation (e.g. 'depth:10')"
    )]
    pub l1_message_inclusion_mode: L1MessageInclusionMode,
}

/// The arguments for the signer.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct SignerArgs {
    /// Path to the file containing the signer's private key
    #[arg(
        long = "signer.key-file",
        value_name = "FILE_PATH",
        help = "Path to the hex-encoded private key file for the signer (optional 0x prefix). Mutually exclusive with AWS KMS key ID"
    )]
    pub key_file: Option<PathBuf>,

    /// AWS KMS Key ID for signing transactions
    #[arg(
        long = "signer.aws-kms-key-id",
        value_name = "KEY_ID",
        help = "AWS KMS Key ID for signing transactions. Mutually exclusive with key file"
    )]
    pub aws_kms_key_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_validate_sequencer_enabled_without_any_signer_fails() {
        let config = ScrollRollupNodeConfig {
            test: false,
            sequencer_args: SequencerArgs { sequencer_enabled: true, ..Default::default() },
            signer_args: SignerArgs { key_file: None, aws_kms_key_id: None },
            database_args: DatabaseArgs::default(),
            engine_driver_args: EngineDriverArgs::default(),
            l1_provider_args: L1ProviderArgs::default(),
            beacon_provider_args: BeaconProviderArgs::default(),
            network_args: NetworkArgs::default(),
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains(
            "Either signer key file or AWS KMS key ID is required when sequencer is enabled"
        ));
    }

    #[test]
    fn test_validate_sequencer_enabled_with_both_signers_fails() {
        let config = ScrollRollupNodeConfig {
            test: false,
            sequencer_args: SequencerArgs { sequencer_enabled: true, ..Default::default() },
            signer_args: SignerArgs {
                key_file: Some(PathBuf::from("/path/to/key")),
                aws_kms_key_id: Some("key-id".to_string()),
            },
            database_args: DatabaseArgs::default(),
            engine_driver_args: EngineDriverArgs::default(),
            l1_provider_args: L1ProviderArgs::default(),
            beacon_provider_args: BeaconProviderArgs::default(),
            network_args: NetworkArgs::default(),
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Cannot specify both signer key file and AWS KMS key ID"));
    }

    #[test]
    fn test_validate_sequencer_enabled_with_key_file_succeeds() {
        let config = ScrollRollupNodeConfig {
            test: false,
            sequencer_args: SequencerArgs { sequencer_enabled: true, ..Default::default() },
            signer_args: SignerArgs {
                key_file: Some(PathBuf::from("/path/to/key")),
                aws_kms_key_id: None,
            },
            database_args: DatabaseArgs::default(),
            engine_driver_args: EngineDriverArgs::default(),
            l1_provider_args: L1ProviderArgs::default(),
            beacon_provider_args: BeaconProviderArgs::default(),
            network_args: NetworkArgs::default(),
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_sequencer_enabled_with_aws_kms_succeeds() {
        let config = ScrollRollupNodeConfig {
            test: false,
            sequencer_args: SequencerArgs { sequencer_enabled: true, ..Default::default() },
            signer_args: SignerArgs { key_file: None, aws_kms_key_id: Some("key-id".to_string()) },
            database_args: DatabaseArgs::default(),
            engine_driver_args: EngineDriverArgs::default(),
            l1_provider_args: L1ProviderArgs::default(),
            beacon_provider_args: BeaconProviderArgs::default(),
            network_args: NetworkArgs::default(),
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_test_mode_without_any_signer_succeeds() {
        let config = ScrollRollupNodeConfig {
            test: true,
            sequencer_args: SequencerArgs { sequencer_enabled: true, ..Default::default() },
            signer_args: SignerArgs { key_file: None, aws_kms_key_id: None },
            database_args: DatabaseArgs::default(),
            engine_driver_args: EngineDriverArgs::default(),
            l1_provider_args: L1ProviderArgs::default(),
            beacon_provider_args: BeaconProviderArgs::default(),
            network_args: NetworkArgs::default(),
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_sequencer_disabled_without_any_signer_succeeds() {
        let config = ScrollRollupNodeConfig {
            test: false,
            sequencer_args: SequencerArgs { sequencer_enabled: false, ..Default::default() },
            signer_args: SignerArgs { key_file: None, aws_kms_key_id: None },
            database_args: DatabaseArgs::default(),
            engine_driver_args: EngineDriverArgs::default(),
            l1_provider_args: L1ProviderArgs::default(),
            beacon_provider_args: BeaconProviderArgs::default(),
            network_args: NetworkArgs::default(),
        };

        assert!(config.validate().is_ok());
    }
}
