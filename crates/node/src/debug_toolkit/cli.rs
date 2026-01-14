//! CLI subcommand for the debug toolkit.

use crate::test_utils::TestFixtureBuilder;
use alloy_primitives::Address;
use clap::Parser;
use reth_network_peers::TrustedPeer;
use std::{path::PathBuf, str::FromStr};

/// Debug toolkit CLI arguments.
#[derive(Debug, Parser)]
#[command(name = "scroll-debug", about = "Scroll Debug Toolkit - Interactive REPL for debugging")]
pub struct DebugArgs {
    /// Chain to use (dev, scroll-sepolia, scroll-mainnet) or path to genesis file.
    #[arg(long, default_value = "dev")]
    pub chain: String,

    /// Enable sequencer mode.
    #[arg(long)]
    pub sequencer: bool,

    /// Number of follower nodes.
    #[arg(long, default_value = "0")]
    pub followers: usize,

    /// Persistent data directory (uses temp dir if not specified).
    #[arg(long)]
    pub datadir: Option<PathBuf>,

    /// Block time in milliseconds (0 = manual block building only).
    #[arg(long, default_value = "0")]
    pub block_time: u64,

    /// Allow building empty blocks (default: true when sequencer is enabled).
    #[arg(long, default_value = "true")]
    pub allow_empty_blocks: bool,

    /// L1 message inclusion delay in blocks (0 = immediate).
    #[arg(long, default_value = "0")]
    pub l1_message_delay: u64,

    /// L1 RPC endpoint URL (optional, uses mock L1 if not specified).
    #[arg(long)]
    pub l1_url: Option<reqwest::Url>,

    /// Comma-separated list of bootnode enode URLs to connect to.
    #[arg(long, value_delimiter = ',')]
    pub bootnodes: Option<Vec<String>>,

    /// The valid signer address for the network.
    #[arg(long)]
    pub valid_signer: Option<Address>,
}

impl DebugArgs {
    /// Run the debug toolkit with these arguments.
    pub async fn run(self) -> eyre::Result<()> {
        use super::DebugRepl;

        // Build the fixture
        let mut builder = TestFixtureBuilder::new().with_chain(&self.chain)?;

        if self.sequencer {
            builder = builder.sequencer();
        }

        if self.followers > 0 {
            builder = builder.followers(self.followers);
        }

        if self.valid_signer.is_some() {
            builder = builder.with_consensus_system_contract(self.valid_signer);
            builder = builder.with_network_valid_signer(self.valid_signer);
        }

        if self.bootnodes.as_ref().map(|b| !b.is_empty()).unwrap_or(false) ||
            self.l1_url.is_some() ||
            self.valid_signer.is_some()
        {
            // Disable test mode if bootnodes or l1 url are specified
            builder.config_mut().test = false;
        }

        // Apply sequencer settings
        builder = builder
            .block_time(self.block_time)
            .allow_empty_blocks(self.allow_empty_blocks)
            .with_l1_message_delay(self.l1_message_delay);

        // Apply L1 URL if provided
        if let Some(l1_url) = self.l1_url {
            let config = builder.config_mut();
            config.l1_provider_args.url = Some(l1_url);
        }

        // Parse and apply bootnodes if provided
        if let Some(bootnode_strs) = self.bootnodes {
            let mut bootnodes = Vec::with_capacity(bootnode_strs.len());
            for enode in bootnode_strs {
                match TrustedPeer::from_str(&enode) {
                    Ok(peer) => bootnodes.push(peer),
                    Err(e) => {
                        return Err(eyre::eyre!("Failed to parse bootnode '{}': {}", enode, e));
                    }
                }
            }
            if !bootnodes.is_empty() {
                builder = builder.bootnodes(bootnodes);
            }
        }

        let fixture = builder.build().await?;

        // Create and run REPL
        let mut repl = DebugRepl::new(fixture);
        repl.run().await
    }
}

/// Entry point for the debug toolkit.
///
/// Usage:
/// ```bash
/// cargo run --features debug-toolkit --bin scroll-debug -- --chain dev --sequencer
/// ```
pub async fn main() -> eyre::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Parse arguments and run
    let args = DebugArgs::parse();
    args.run().await
}
