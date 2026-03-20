/// REPL for attaching to an already-running scroll node via JSON-RPC.
use crate::debug_toolkit::commands::{
    print_help, AdminCommand, BlockArg, Command, EventsCommand, L1Command, PeersCommand, TxCommand,
};
use alloy_consensus::{SignableTransaction, TxEip1559};
use alloy_eips::{eip2718::Encodable2718, BlockId, BlockNumberOrTag};
use alloy_network::TxSignerSync;
use alloy_primitives::TxKind;
use alloy_provider::{Provider, ProviderBuilder};
use alloy_signer_local::PrivateKeySigner;
use colored::Colorize;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use reqwest::Url;
use rollup_node_chain_orchestrator::ChainOrchestratorStatus;
use scroll_alloy_network::Scroll;
use std::{io::Write, path::PathBuf, time::Duration};

/// Interactive REPL that attaches to a running node via JSON-RPC.
#[derive(Debug)]
pub struct AttachRepl {
    /// The RPC URL of the target node.
    url: Url,
    /// Alloy provider — all RPC calls including custom namespaces go through `raw_request`.
    provider: alloy_provider::RootProvider<Scroll>,
    /// Optional private key for signing transactions locally.
    signer: Option<PrivateKeySigner>,
    /// Whether the REPL is running.
    running: bool,
    /// Whether background head-block polling is enabled.
    events_enabled: bool,
    /// Most recently seen block number (for head-block polling).
    last_seen_block: u64,
    /// Path to the log file (for `logs` command).
    log_path: Option<PathBuf>,
}

impl AttachRepl {
    /// Connect to a node at the given URL and build the REPL.
    pub async fn new(url: Url, private_key: Option<String>) -> eyre::Result<Self> {
        // Use `default()` (no fillers) to get a plain `RootProvider<Ethereum>`.
        // We don't need gas/nonce fillers since we build transactions manually.
        let provider = ProviderBuilder::default()
            .connect(url.as_str())
            .await
            .map_err(|e| eyre::eyre!("Failed to connect to {}: {}", url, e))?;

        let signer = if let Some(pk) = private_key {
            let pk = pk.trim_start_matches("0x");
            let signer: PrivateKeySigner =
                pk.parse().map_err(|e| eyre::eyre!("Invalid private key: {}", e))?;
            Some(signer)
        } else {
            None
        };

        let last_seen_block = provider.get_block_number().await.unwrap_or(0);

        Ok(Self {
            url,
            provider,
            signer,
            running: false,
            events_enabled: false,
            last_seen_block,
            log_path: None,
        })
    }

    /// Set the log file path (shown by `logs` command).
    pub fn set_log_path(&mut self, path: PathBuf) {
        self.log_path = Some(path);
    }

    /// Get the REPL prompt string.
    fn get_prompt(&self) -> String {
        let host = self.url.host_str().unwrap_or("?");
        let port = self.url.port().map(|p| format!(":{}", p)).unwrap_or_default();
        format!("{} [{}{}]> ", "scroll-debug".cyan(), host, port)
    }

    /// Run the REPL loop.
    pub async fn run(&mut self) -> eyre::Result<()> {
        self.running = true;

        let _guard = super::terminal::RawModeGuard::new()?;

        let _ = disable_raw_mode();
        println!();
        println!("{}", "Scroll Debug Toolkit (attach mode)".bold().cyan());
        println!("Connected to: {}", self.url.as_str().green());
        if let Some(signer) = &self.signer {
            println!("Signer:       {:?}", signer.address());
        } else {
            println!("{}", "No signer, tx send/inject require --private-key".yellow());
        }
        println!("Type 'help' for available commands, 'exit' to quit.");
        println!();
        if let Err(e) = self.cmd_status().await {
            println!("{}: {}", "Warning: could not fetch initial status".yellow(), e);
        }
        let _ = enable_raw_mode();

        let mut input_buffer = String::new();
        let mut stdout = std::io::stdout();
        let mut head_poll_tick = tokio::time::interval(Duration::from_secs(2));
        head_poll_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut input_tick = tokio::time::interval(Duration::from_millis(50));
        input_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        print!("{}", self.get_prompt());
        let _ = stdout.flush();

        while self.running {
            tokio::select! {
                biased;

                // Head-block polling (only when events are enabled).
                _ = head_poll_tick.tick(), if self.events_enabled => {
                    if let Ok(number) = self.provider.get_block_number().await {
                        if number > self.last_seen_block {
                            for n in (self.last_seen_block + 1)..=number {
                                let id = BlockId::Number(BlockNumberOrTag::Number(n));
                                if let Ok(Some(block)) = self.provider.get_block(id).await {
                                    let msg = format!(
                                        "[new block] #{} hash={:.12}... txs={}",
                                        block.header.number,
                                        format!("{:?}", block.header.hash),
                                        block.transactions.len(),
                                    );
                                    print!("\r\x1b[K{}\r\n{}{}", msg.cyan(), self.get_prompt(), input_buffer);
                                    let _ = stdout.flush();
                                }
                            }
                            self.last_seen_block = number;
                        }
                    }
                }

                // Check for keyboard input (non-blocking)
                _ = input_tick.tick() => {
                    match super::terminal::poll_keyboard(&mut input_buffer, &self.get_prompt())? {
                        super::terminal::InputAction::Command(line) => {
                            let _ = disable_raw_mode();
                            if let Err(e) = self.execute_command(&line).await {
                                println!("{}: {}", "Error".red(), e);
                            }
                            let _ = enable_raw_mode();
                            if self.running {
                                print!("{}", self.get_prompt());
                                let _ = stdout.flush();
                            }
                        }
                        super::terminal::InputAction::Quit => self.running = false,
                        super::terminal::InputAction::None => {}
                    }
                }
            }
        }

        print!("Goodbye!\r\n");
        Ok(())
    }

    /// Dispatch a parsed command.
    async fn execute_command(&mut self, input: &str) -> eyre::Result<()> {
        let cmd = Command::parse(input);
        match cmd {
            Command::Status => self.cmd_status().await,
            Command::SyncStatus => self.cmd_sync_status().await,
            Command::Block(arg) => self.cmd_block(arg).await,
            Command::Blocks { from, to } => self.cmd_blocks(from, to).await,
            Command::Fcs => self.cmd_fcs().await,
            Command::L1(l1_cmd) => self.cmd_l1(l1_cmd).await,
            Command::Tx(tx_cmd) => self.cmd_tx(tx_cmd).await,
            Command::Peers(peers_cmd) => self.cmd_peers(peers_cmd).await,
            Command::Events(events_cmd) => self.cmd_events(events_cmd),
            Command::Admin(admin_cmd) => self.cmd_admin(admin_cmd).await,
            Command::Rpc { method, params } => self.cmd_rpc(&method, params.as_deref()).await,
            Command::Logs => self.cmd_logs(),
            Command::Help => {
                print_help();
                Ok(())
            }
            Command::Exit => {
                self.running = false;
                Ok(())
            }
            // Spawn-mode-only commands — give informative errors
            Command::Build => {
                println!(
                    "{}",
                    "build is only available in spawn mode (--chain / --sequencer).".yellow()
                );
                Ok(())
            }
            Command::Wallet(_) => {
                println!(
                    "{}",
                    "wallet gen is only available in spawn mode. Use --private-key to set a signer."
                        .yellow()
                );
                Ok(())
            }
            Command::Run(_) => {
                println!("{}", "run actions are only available in spawn mode.".yellow());
                Ok(())
            }
            Command::Node(_) | Command::Nodes => {
                println!(
                    "{}",
                    "node switching is only available in spawn mode (multiple nodes).".yellow()
                );
                Ok(())
            }
            Command::Db => {
                println!("{}", "db path is only available in spawn mode.".yellow());
                Ok(())
            }
            Command::Unknown(s) => {
                if !s.is_empty() {
                    println!("Unknown command: {}. Type 'help' for available commands.", s);
                }
                Ok(())
            }
        }
    }

    // -------------------------------------------------------------------------
    // Helper
    // -------------------------------------------------------------------------

    /// Call a custom-namespace JSON-RPC method and deserialize the response.
    ///
    /// Uses `raw_request_dyn` (no trait bounds on P/R) combined with `serde_json` for
    /// maximum compatibility regardless of the provider's network/transport generics.
    async fn raw<R: serde::de::DeserializeOwned>(
        &self,
        method: &'static str,
        params: impl serde::Serialize,
    ) -> eyre::Result<R> {
        crate::debug_toolkit::shared::rpc::raw_typed(&self.provider, method, params).await
    }

    // -------------------------------------------------------------------------
    // Command implementations
    // -------------------------------------------------------------------------

    /// `status` — show node status via `rollupNode_status`.
    async fn cmd_status(&self) -> eyre::Result<()> {
        let status: ChainOrchestratorStatus = self.raw("rollupNode_status", ()).await?;

        println!("{}", "=== Node Status ===".bold());
        println!("{}", "Node:".underline());
        println!("  RPC:  {}", self.url.as_str());
        if let Some(signer) = &self.signer {
            println!("  From: {:?}", signer.address());
        }
        crate::debug_toolkit::shared::status::print_status_overview(&status);

        Ok(())
    }

    /// `sync-status` — detailed sync status.
    async fn cmd_sync_status(&self) -> eyre::Result<()> {
        let status: ChainOrchestratorStatus = self.raw("rollupNode_status", ()).await?;
        crate::debug_toolkit::shared::status::print_sync_status(&status);
        Ok(())
    }

    /// `fcs` — show forkchoice state.
    async fn cmd_fcs(&self) -> eyre::Result<()> {
        let status: ChainOrchestratorStatus = self.raw("rollupNode_status", ()).await?;
        crate::debug_toolkit::shared::status::print_forkchoice(&status);
        Ok(())
    }

    /// `block [n|latest]` — show block details.
    async fn cmd_block(&self, arg: BlockArg) -> eyre::Result<()> {
        let tag = match arg {
            BlockArg::Latest => BlockNumberOrTag::Latest,
            BlockArg::Number(n) => BlockNumberOrTag::Number(n),
        };

        let block: Option<serde_json::Value> =
            self.raw("eth_getBlockByNumber", (tag, false)).await?;
        let block = block.ok_or_else(|| eyre::eyre!("Block not found"))?;

        let number = block["number"].as_str().unwrap_or("?");
        let hash = block["hash"].as_str().unwrap_or("?");
        let parent = block["parentHash"].as_str().unwrap_or("?");
        let timestamp = block["timestamp"].as_str().unwrap_or("?");
        let gas_used = block["gasUsed"].as_str().unwrap_or("?");
        let gas_limit = block["gasLimit"].as_str().unwrap_or("?");
        let txs = block["transactions"].as_array();

        println!("{}", format!("Block {}", number).bold());
        println!("  Hash:       {}", hash);
        println!("  Parent:     {}", parent);
        println!("  Timestamp:  {}", timestamp);
        println!("  Gas Used:   {}", gas_used);
        println!("  Gas Limit:  {}", gas_limit);

        if let Some(txs) = txs {
            println!("  Txs:        {}", txs.len());
            for (i, tx) in txs.iter().enumerate() {
                let tx_hash = tx.as_str().or_else(|| tx["hash"].as_str()).unwrap_or("?");
                println!("    [{}] hash={}", i, tx_hash);
            }
        }

        Ok(())
    }

    /// `blocks <from> <to>` — list blocks in a range.
    async fn cmd_blocks(&self, from: u64, to: u64) -> eyre::Result<()> {
        println!("{}", format!("Blocks {} to {}:", from, to).bold());
        for n in from..=to {
            let tag = BlockNumberOrTag::Number(n);
            let block: Option<serde_json::Value> =
                self.raw("eth_getBlockByNumber", (tag, false)).await?;
            if let Some(block) = block {
                let hash = block["hash"].as_str().unwrap_or("?");
                let gas = block["gasUsed"].as_str().unwrap_or("?");
                let tx_count = block["transactions"].as_array().map(|a| a.len()).unwrap_or(0);
                println!("  #{}: {} txs, gas: {}, hash: {:.12}...", n, tx_count, gas, hash);
            } else {
                println!("  #{}: {}", n, "not found".dimmed());
            }
        }
        Ok(())
    }

    /// `l1 status` / `l1 messages` — L1-related queries.
    async fn cmd_l1(&self, cmd: L1Command) -> eyre::Result<()> {
        match cmd {
            L1Command::Status => {
                let status: ChainOrchestratorStatus = self.raw("rollupNode_status", ()).await?;
                println!("{}", "L1 Status:".bold());
                println!(
                    "  Synced:    {}",
                    if status.l1.status.is_synced() { "true".green() } else { "false".red() }
                );
                println!("  L1 Head:   #{}", status.l1.latest);
                println!("  L1 Final:  #{}", status.l1.finalized);
                println!("  Processed: #{}", status.l1.processed);
            }
            L1Command::Messages => {
                let msg: Option<serde_json::Value> =
                    self.raw("rollupNode_getL1MessageByIndex", [0u64]).await?;
                println!("{}", "L1 Message Queue (index 0):".bold());
                match msg {
                    Some(m) => println!("{}", serde_json::to_string_pretty(&m)?),
                    None => println!("  {}", "No message at index 0".dimmed()),
                }
                println!(
                    "{}",
                    "Hint: use 'rpc rollupNode_getL1MessageByIndex [<index>]' for specific indices"
                        .dimmed()
                );
            }
            L1Command::Sync | L1Command::Block(_) | L1Command::Reorg(_) => {
                println!(
                    "{}",
                    "l1 sync/block/reorg are only available in spawn mode (mock L1).".yellow()
                );
            }
            L1Command::Send { .. } => {
                println!(
                    "{}",
                    "l1 send is only available in spawn mode. Use cast or a wallet to bridge."
                        .yellow()
                );
            }
        }
        Ok(())
    }

    /// `tx pending` / `tx send` / `tx inject`.
    async fn cmd_tx(&self, cmd: TxCommand) -> eyre::Result<()> {
        match cmd {
            TxCommand::Pending => {
                let result: serde_json::Value = self.raw("txpool_content", ()).await?;
                println!("{}", "Pending Transactions:".bold());
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            TxCommand::Send { to, value, from: _ } => {
                let signer = self.signer.as_ref().ok_or_else(|| {
                    eyre::eyre!("No signer configured. Start with --private-key <hex>.")
                })?;
                let from_address = signer.address();

                let chain_id: serde_json::Value = self.raw("eth_chainId", ()).await?;
                let chain_id: u64 = u64::from_str_radix(
                    chain_id.as_str().unwrap_or("0x1").trim_start_matches("0x"),
                    16,
                )
                .unwrap_or(1);

                let nonce_val: serde_json::Value =
                    self.raw("eth_getTransactionCount", (from_address, "latest")).await?;
                let nonce: u64 = u64::from_str_radix(
                    nonce_val.as_str().unwrap_or("0x0").trim_start_matches("0x"),
                    16,
                )
                .unwrap_or(0);

                let latest: serde_json::Value =
                    self.raw("eth_getBlockByNumber", ("latest", false)).await?;
                let base_fee = latest["baseFeePerGas"]
                    .as_str()
                    .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
                    .unwrap_or(1_000_000_000);
                let base_fee_u128 = base_fee as u128;
                // Keep priority tip conservative on low-fee chains and always satisfy:
                // max_fee_per_gas >= max_priority_fee_per_gas.
                let max_priority_fee_per_gas = (base_fee_u128 / 2).max(1);
                let max_fee_per_gas = (base_fee_u128 * 2).max(max_priority_fee_per_gas);
                let gas_limit = match self
                    .raw::<serde_json::Value>(
                        "eth_estimateGas",
                        [serde_json::json!({
                            "from": format!("{:#x}", from_address),
                            "to": format!("{:#x}", to),
                            "value": format!("0x{value:x}"),
                        })],
                    )
                    .await
                {
                    Ok(v) => v
                        .as_str()
                        .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
                        // Add a small safety buffer on top of estimate.
                        .map(|g| g.saturating_mul(12) / 10)
                        .filter(|g| *g > 0)
                        .unwrap_or(21_000),
                    Err(e) => {
                        println!(
                            "{}",
                            format!(
                                "Warning: eth_estimateGas failed ({}), falling back to 21000",
                                e
                            )
                            .yellow()
                        );
                        21_000
                    }
                };

                let mut tx = TxEip1559 {
                    chain_id,
                    nonce,
                    gas_limit,
                    max_fee_per_gas,
                    max_priority_fee_per_gas,
                    to: TxKind::Call(to),
                    value,
                    access_list: Default::default(),
                    input: Default::default(),
                };

                let signature = signer.sign_transaction_sync(&mut tx)?;
                let signed = tx.into_signed(signature);
                let raw_tx = alloy_primitives::hex::encode_prefixed(signed.encoded_2718());

                let tx_hash: serde_json::Value =
                    self.raw("eth_sendRawTransaction", [raw_tx]).await?;
                let tx_hash_str =
                    tx_hash.as_str().map(ToOwned::to_owned).unwrap_or_else(|| tx_hash.to_string());
                crate::debug_toolkit::shared::output::print_tx_sent(
                    &tx_hash_str,
                    &format!("{:?}", from_address),
                    &format!("{:?}", to),
                    value,
                    false,
                );
            }
            TxCommand::Inject(bytes) => {
                let hex = alloy_primitives::hex::encode_prefixed(&bytes);
                let tx_hash: serde_json::Value = self.raw("eth_sendRawTransaction", [hex]).await?;
                let tx_hash_str =
                    tx_hash.as_str().map(ToOwned::to_owned).unwrap_or_else(|| tx_hash.to_string());
                crate::debug_toolkit::shared::output::print_tx_injected(&tx_hash_str);
            }
        }
        Ok(())
    }

    /// `peers` / `peers connect`.
    async fn cmd_peers(&self, cmd: PeersCommand) -> eyre::Result<()> {
        match cmd {
            PeersCommand::List => {
                let peers: serde_json::Value = self.raw("admin_peers", ()).await?;
                println!("{}", "Connected Peers:".bold());
                println!("{}", serde_json::to_string_pretty(&peers)?);

                println!();
                let node_info: serde_json::Value = self.raw("admin_nodeInfo", ()).await?;
                println!("{}", "Local Node Info:".bold());
                println!("{}", serde_json::to_string_pretty(&node_info)?);
            }
            PeersCommand::Connect(enode_url) => {
                let result: bool = self.raw("admin_addPeer", [enode_url.as_str()]).await?;
                if result {
                    println!("{}", format!("Peer add request sent: {}", enode_url).green());
                } else {
                    println!("{}", "admin_addPeer returned false".yellow());
                }
            }
        }
        Ok(())
    }

    /// `events on` / `events off` — toggle head-block polling.
    fn cmd_events(&mut self, cmd: EventsCommand) -> eyre::Result<()> {
        match cmd {
            EventsCommand::On => {
                self.events_enabled = true;
                println!("{}", "Head-block polling enabled (2s interval)".green());
            }
            EventsCommand::Off => {
                self.events_enabled = false;
                println!("{}", "Head-block polling disabled".yellow());
            }
            EventsCommand::Filter(_) | EventsCommand::History(_) => {
                println!("{}", "events filter/history are only available in spawn mode.".yellow());
            }
        }
        Ok(())
    }

    /// `admin enable-seq` / `admin disable-seq` / `admin revert <n>`.
    async fn cmd_admin(&self, cmd: AdminCommand) -> eyre::Result<()> {
        match cmd {
            AdminCommand::EnableSequencing => {
                let result: bool =
                    self.raw("rollupNodeAdmin_enableAutomaticSequencing", ()).await?;
                crate::debug_toolkit::shared::output::print_admin_enable_result(result);
            }
            AdminCommand::DisableSequencing => {
                let result: bool =
                    self.raw("rollupNodeAdmin_disableAutomaticSequencing", ()).await?;
                crate::debug_toolkit::shared::output::print_admin_disable_result(result);
            }
            AdminCommand::RevertToL1Block(block_number) => {
                crate::debug_toolkit::shared::output::print_admin_revert_start(block_number);
                let result: bool =
                    self.raw("rollupNodeAdmin_revertToL1Block", [block_number]).await?;
                crate::debug_toolkit::shared::output::print_admin_revert_result(
                    block_number,
                    result,
                );
            }
        }
        Ok(())
    }

    /// `rpc <method> [params]` — call any JSON-RPC method and pretty-print the result.
    async fn cmd_rpc(&self, method: &str, params: Option<&str>) -> eyre::Result<()> {
        let pretty =
            crate::debug_toolkit::shared::rpc::raw_value(&self.provider, method, params).await?;
        crate::debug_toolkit::shared::output::print_pretty_json(&pretty)
    }

    /// `logs` — show log file path.
    fn cmd_logs(&self) -> eyre::Result<()> {
        crate::debug_toolkit::shared::output::print_log_file(&self.log_path);
        Ok(())
    }
}
