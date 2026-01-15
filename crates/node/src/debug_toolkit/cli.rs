//! CLI subcommand for the debug toolkit.

use crate::{test_utils::TestFixtureBuilder, L1ProviderArgs};
use alloy_primitives::Address;
use alloy_provider::{layers::CacheLayer, ProviderBuilder};
use alloy_rpc_client::RpcClient;
use alloy_transport::layers::RetryBackoffLayer;
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

    /// L1 RPC endpoint URL (optional, uses mock L1 if not specified).
    #[arg(long)]
    pub l1_url: Option<reqwest::Url>,

    /// Comma-separated list of bootnode enode URLs to connect to.
    #[arg(long, value_delimiter = ',')]
    pub bootnodes: Option<Vec<String>>,

    /// The valid signer address for the network.
    #[arg(long)]
    pub valid_signer: Option<Address>,

    /// Path to log file. Defaults to ./scroll-debug-<pid>.log
    #[arg(long)]
    pub log_file: Option<PathBuf>,
}

impl DebugArgs {
    /// Run the debug toolkit with these arguments.
    pub async fn run(self, log_path: Option<PathBuf>) -> eyre::Result<()> {
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

        // Apply L1 URL if provided - build provider for REPL access
        if let Some(l1_url) = self.l1_url {
            builder.config_mut().l1_provider_args.url = Some(l1_url.clone());

            // Build the L1 provider with retry and cache layers
            let L1ProviderArgs {
                max_retries,
                initial_backoff,
                compute_units_per_second,
                cache_max_items,
                ..
            } = L1ProviderArgs::default();

            let client = RpcClient::builder()
                .layer(RetryBackoffLayer::new(
                    max_retries,
                    initial_backoff,
                    compute_units_per_second,
                ))
                .http(l1_url);
            let cache_layer = CacheLayer::new(cache_max_items);
            let provider = ProviderBuilder::new().layer(cache_layer).connect_client(client);

            builder = builder.with_l1_provider(Box::new(provider));
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
        if let Some(path) = log_path {
            repl.set_log_path(path);
        }
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
    args.run(None).await
}
