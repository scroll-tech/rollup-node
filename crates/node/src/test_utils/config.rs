//! Test configuration builder for creating rollup node test configurations.

use crate::{
    BlobProviderArgs, ChainOrchestratorArgs, ConsensusArgs, EngineDriverArgs, L1ProviderArgs,
    RollupNodeDatabaseArgs, RollupNodeGasPriceOracleArgs, RollupNodeNetworkArgs, RpcArgs,
    ScrollRollupNodeConfig, SequencerArgs,
};
use alloy_primitives::Address;
use rollup_node_sequencer::L1MessageInclusionMode;
use std::path::PathBuf;

/// Builder for creating test configurations with a fluent API.
#[derive(Debug)]
pub struct TestConfigBuilder {
    config: ScrollRollupNodeConfig,
}

impl Default for TestConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TestConfigBuilder {
    /// Create a new test configuration builder with sensible defaults.
    pub fn new() -> Self {
        Self {
            config: ScrollRollupNodeConfig {
                test: true,
                network_args: RollupNodeNetworkArgs::default(),
                database_args: RollupNodeDatabaseArgs::default(),
                l1_provider_args: L1ProviderArgs::default(),
                engine_driver_args: EngineDriverArgs { sync_at_startup: true },
                chain_orchestrator_args: ChainOrchestratorArgs {
                    optimistic_sync_trigger: 100,
                    chain_buffer_size: 100,
                },
                sequencer_args: SequencerArgs {
                    payload_building_duration: 1000,
                    allow_empty_blocks: true,
                    ..Default::default()
                },
                blob_provider_args: BlobProviderArgs { mock: true, ..Default::default() },
                signer_args: Default::default(),
                gas_price_oracle_args: RollupNodeGasPriceOracleArgs::default(),
                consensus_args: ConsensusArgs::noop(),
                database: None,
                rpc_args: RpcArgs { enabled: true },
            },
        }
    }

    /// Enable sequencer with default settings for testing.
    pub fn with_sequencer(&mut self) -> &mut Self {
        self.config.sequencer_args.sequencer_enabled = true;
        self.config.sequencer_args.auto_start = false;
        self.config.sequencer_args.block_time = 100;
        self.config.sequencer_args.payload_building_duration = 40;
        self.config.sequencer_args.l1_message_inclusion_mode =
            L1MessageInclusionMode::BlockDepth(0);
        self.config.sequencer_args.allow_empty_blocks = true;
        self.config.database_args.rn_db_path = Some(PathBuf::from("sqlite::memory:"));
        self
    }

    /// Set the sequencer block time in milliseconds.
    pub fn block_time(&mut self, millis: u64) -> &mut Self {
        self.config.sequencer_args.block_time = millis;
        self
    }

    /// Set whether to allow empty blocks.
    pub fn allow_empty_blocks(&mut self, allow: bool) -> &mut Self {
        self.config.sequencer_args.allow_empty_blocks = allow;
        self
    }

    /// Set L1 message inclusion mode with block depth.
    pub fn with_l1_message_delay(&mut self, depth: u64) -> &mut Self {
        self.config.sequencer_args.l1_message_inclusion_mode =
            L1MessageInclusionMode::BlockDepth(depth);
        self
    }

    /// Set L1 message inclusion mode to finalized with optional block depth.
    pub fn with_finalized_l1_messages(&mut self, depth: u64) -> &mut Self {
        self.config.sequencer_args.l1_message_inclusion_mode =
            L1MessageInclusionMode::FinalizedWithBlockDepth(depth);
        self
    }

    /// Use an in-memory `SQLite` database.
    pub fn with_memory_db(&mut self) -> &mut Self {
        self.config.database_args.rn_db_path = Some(PathBuf::from("sqlite::memory:"));
        self
    }

    /// Set a custom database path.
    pub fn with_db_path(&mut self, path: PathBuf) -> &mut Self {
        self.config.database_args.rn_db_path = Some(path);
        self
    }

    /// Use noop consensus (no validation).
    pub fn with_noop_consensus(&mut self) -> &mut Self {
        self.config.consensus_args = ConsensusArgs::noop();
        self
    }

    /// Set the payload building duration in milliseconds.
    pub fn payload_building_duration(&mut self, millis: u64) -> &mut Self {
        self.config.sequencer_args.payload_building_duration = millis;
        self
    }

    /// Set the fee recipient address.
    pub fn fee_recipient(&mut self, address: Address) -> &mut Self {
        self.config.sequencer_args.fee_recipient = address;
        self
    }

    /// Enable auto-start for the sequencer.
    pub fn auto_start(&mut self, enabled: bool) -> &mut Self {
        self.config.sequencer_args.auto_start = enabled;
        self
    }

    /// Set the maximum number of L1 messages per block.
    pub fn max_l1_messages(&mut self, max: u64) -> &mut Self {
        self.config.sequencer_args.max_l1_messages = Some(max);
        self
    }

    /// Enable the Scroll wire protocol.
    pub fn with_scroll_wire(&mut self, enabled: bool) -> &mut Self {
        self.config.network_args.enable_scroll_wire = enabled;
        self
    }

    /// Enable the ETH-Scroll wire bridge.
    pub fn with_eth_scroll_bridge(&mut self, enabled: bool) -> &mut Self {
        self.config.network_args.enable_eth_scroll_wire_bridge = enabled;
        self
    }

    /// Set the optimistic sync trigger threshold.
    pub fn optimistic_sync_trigger(&mut self, blocks: u64) -> &mut Self {
        self.config.chain_orchestrator_args.optimistic_sync_trigger = blocks;
        self
    }

    /// Set the chain buffer size.
    pub fn chain_buffer_size(&mut self, size: usize) -> &mut Self {
        self.config.chain_orchestrator_args.chain_buffer_size = size;
        self
    }

    /// Disable the test mode (enables real signing).
    pub fn production_mode(&mut self) -> &mut Self {
        self.config.test = false;
        self
    }

    /// Build the final configuration.
    pub fn build(self) -> ScrollRollupNodeConfig {
        self.config
    }

    /// Get a mutable reference to the underlying config for advanced customization.
    pub fn config_mut(&mut self) -> &mut ScrollRollupNodeConfig {
        &mut self.config
    }
}

/// Convenience function to create a new test config builder.
pub fn test_config() -> TestConfigBuilder {
    TestConfigBuilder::new()
}
