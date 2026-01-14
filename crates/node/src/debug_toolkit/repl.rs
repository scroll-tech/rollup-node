//! Interactive REPL for debugging rollup nodes.

use super::{
    actions::ActionRegistry,
    commands::{
        print_help, BlockArg, Command, EventsCommand, L1Command, PeersCommand, RunCommand,
        TxCommand, WalletCommand,
    },
    event_stream::EventStreamState,
};
use crate::test_utils::{fixture::NodeType, TestFixture};
use alloy_consensus::{SignableTransaction, TxEip1559};
use alloy_eips::{eip2718::Encodable2718, BlockNumberOrTag};
use alloy_network::{TransactionResponse, TxSignerSync};
use alloy_primitives::TxKind;
use colored::Colorize;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use futures::StreamExt;
use reth_network::PeersInfo;
use reth_network_api::Peers;
use reth_network_peers::TrustedPeer;
use reth_rpc_api::EthApiServer;
use reth_transaction_pool::TransactionPool;
use std::{io::Write, str::FromStr, time::Duration};

/// Interactive REPL for debugging rollup nodes.
pub struct DebugRepl {
    /// The test fixture containing nodes.
    fixture: TestFixture,
    /// Whether the REPL is running.
    running: bool,
    /// Current active node index.
    active_node: usize,
    /// Event stream state per node.
    event_streams: Vec<EventStreamState>,
    /// Registry of custom actions.
    action_registry: ActionRegistry,
}

impl std::fmt::Debug for DebugRepl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DebugRepl")
            .field("running", &self.running)
            .field("active_node", &self.active_node)
            .field("event_streams", &self.event_streams)
            .field("action_registry", &"ActionRegistry { ... }")
            .finish_non_exhaustive()
    }
}

impl DebugRepl {
    /// Create a new REPL with the given fixture.
    pub fn new(fixture: TestFixture) -> Self {
        // Create one event stream per node, enabled by default
        let event_streams = (0..fixture.nodes.len())
            .map(|_| {
                let mut es = EventStreamState::new();
                es.enable();
                es
            })
            .collect();

        Self {
            fixture,
            running: false,
            active_node: 0,
            event_streams,
            action_registry: ActionRegistry::new(),
        }
    }

    /// Create a new REPL with a custom action registry.
    pub fn with_action_registry(fixture: TestFixture, action_registry: ActionRegistry) -> Self {
        let event_streams = (0..fixture.nodes.len())
            .map(|_| {
                let mut es = EventStreamState::new();
                es.enable();
                es
            })
            .collect();

        Self { fixture, running: false, active_node: 0, event_streams, action_registry }
    }

    /// Get mutable access to the action registry to register custom actions.
    pub const fn action_registry_mut(&mut self) -> &mut ActionRegistry {
        &mut self.action_registry
    }

    /// Run the REPL loop.
    pub async fn run(&mut self) -> eyre::Result<()> {
        self.running = true;

        // Print welcome message
        println!();
        println!("{}", "Scroll Debug Toolkit".bold().cyan());
        println!("Type 'help' for available commands, 'exit' to quit.");
        println!();

        // Show initial status
        self.cmd_status().await?;

        // Current input line buffer
        let mut input_buffer = String::new();
        let mut stdout = std::io::stdout();

        // Print initial prompt
        print!("{}", self.get_prompt());
        let _ = stdout.flush();

        while self.running {
            // Poll for events and input
            tokio::select! {
                biased;

                // Check for events from the node
                Some(event) = self.fixture.nodes[self.active_node].chain_orchestrator_rx.next() => {
                    // Display if streaming is enabled
                    if let Some(formatted) = self.event_streams[self.active_node].record_event(event) {
                        // Clear current line, print event, reprint prompt
                        print!("\r\x1b[K{}\n{}{}", formatted, self.get_prompt(), input_buffer);
                        let _ = stdout.flush();
                    }
                }

                // Check for keyboard input (non-blocking)
                _ = tokio::time::sleep(Duration::from_millis(50)) => {
                    // Poll for keyboard events
                    while event::poll(Duration::from_millis(0))? {
                        if let Event::Key(key_event) = event::read()? {
                            match key_event.code {
                                KeyCode::Enter => {
                                    println!();
                                    let line = input_buffer.trim().to_string();
                                    input_buffer.clear();

                                    if !line.is_empty() {
                                        if let Err(e) = self.execute_command(&line).await {
                                            println!("{}: {}", "Error".red(), e);
                                        }
                                    }

                                    if self.running {
                                        print!("{}", self.get_prompt());
                                        let _ = stdout.flush();
                                    }
                                }
                                KeyCode::Backspace => {
                                    if !input_buffer.is_empty() {
                                        input_buffer.pop();
                                        print!("\x08 \x08"); // Move back, overwrite, move back
                                        let _ = stdout.flush();
                                    }
                                }
                                KeyCode::Char(c) => {
                                    if key_event.modifiers.contains(KeyModifiers::CONTROL) && c == 'c' {
                                        println!("\nUse 'exit' to quit");
                                        print!("{}{}", self.get_prompt(), input_buffer);
                                        let _ = stdout.flush();
                                    } else if key_event.modifiers.contains(KeyModifiers::CONTROL) && c == 'd' {
                                        println!();
                                        self.running = false;
                                    } else {
                                        input_buffer.push(c);
                                        print!("{}", c);
                                        let _ = stdout.flush();
                                    }
                                }
                                KeyCode::Esc => {
                                    // Clear current input
                                    print!("\r\x1b[K{}", self.get_prompt());
                                    let _ = stdout.flush();
                                    input_buffer.clear();
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        println!("Goodbye!");
        Ok(())
    }

    /// Get the REPL prompt.
    fn get_prompt(&self) -> String {
        let node_type = match self.fixture.nodes[self.active_node].typ {
            NodeType::Sequencer => "seq",
            NodeType::Follower => "fol",
        };
        format!("{} [{}:{}]> ", "scroll-debug".cyan(), node_type, self.active_node)
    }

    /// Execute a command.
    async fn execute_command(&mut self, input: &str) -> eyre::Result<()> {
        let cmd = Command::parse(input);

        match cmd {
            Command::Status => self.cmd_status().await,
            Command::Block(arg) => self.cmd_block(arg).await,
            Command::Blocks { from, to } => self.cmd_blocks(from, to).await,
            Command::Fcs => self.cmd_fcs().await,
            Command::L1(l1_cmd) => self.cmd_l1(l1_cmd).await,
            Command::Build => self.cmd_build().await,
            Command::Tx(tx_cmd) => self.cmd_tx(tx_cmd).await,
            Command::Wallet(wallet_cmd) => self.cmd_wallet(wallet_cmd).await,
            Command::Peers(peers_cmd) => self.cmd_peers(peers_cmd).await,
            Command::Events(events_cmd) => self.cmd_events(events_cmd).await,
            Command::Run(run_cmd) => self.cmd_run(run_cmd).await,
            Command::Node(idx) => self.cmd_switch_node(idx),
            Command::Nodes => self.cmd_list_nodes(),
            Command::Db => self.cmd_db(),
            Command::Help => {
                print_help();
                Ok(())
            }
            Command::Exit => {
                self.running = false;
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

    /// Show node status.
    async fn cmd_status(&self) -> eyre::Result<()> {
        let node = &self.fixture.nodes[self.active_node];
        let node_type = match node.typ {
            NodeType::Sequencer => "Sequencer",
            NodeType::Follower => "Follower",
        };

        let status = node.rollup_manager_handle.status().await?;
        let fcs = &status.l2.fcs;

        println!("{}", format!("=== Node {} ({}) ===", self.active_node, node_type).bold());

        // Node Info
        let db_path = node.node.inner.config.datadir().db().join("scroll.db");
        let http_addr = node.node.inner.rpc_server_handle().http_local_addr();
        println!("{}", "Node:".underline());
        println!("  Database:  {}", db_path.display());
        if let Some(addr) = http_addr {
            println!("  HTTP RPC:  http://{}", addr);
        }

        // L2 Status
        println!("{}", "L2:".underline());
        println!(
            "  Head:      #{} ({:.12}...)",
            fcs.head_block_info().number.to_string().green(),
            format!("{:?}", fcs.head_block_info().hash)
        );
        println!(
            "  Safe:      #{} ({:.12}...)",
            fcs.safe_block_info().number.to_string().yellow(),
            format!("{:?}", fcs.safe_block_info().hash)
        );
        println!(
            "  Finalized: #{} ({:.12}...)",
            fcs.finalized_block_info().number.to_string().blue(),
            format!("{:?}", fcs.finalized_block_info().hash)
        );
        println!(
            "  Synced:    {}",
            if status.l2.status.is_synced() { "true".green() } else { "false".red() }
        );

        // L1 Status
        println!("{}", "L1:".underline());
        println!("  Head:      #{}", status.l1.latest.to_string().cyan());
        println!("  Finalized: #{}", status.l1.finalized);
        println!("  Processed: #{}", status.l1.processed);
        println!(
            "  Synced:    {}",
            if status.l1.status.is_synced() { "true".green() } else { "false".red() }
        );

        Ok(())
    }

    /// Show block details.
    async fn cmd_block(&self, arg: BlockArg) -> eyre::Result<()> {
        let node = &self.fixture.nodes[self.active_node];

        let block_id = match arg {
            BlockArg::Latest => BlockNumberOrTag::Latest,
            BlockArg::Number(n) => BlockNumberOrTag::Number(n),
        };

        let block = node
            .node
            .rpc
            .inner
            .eth_api()
            .block_by_number(block_id, true)
            .await?
            .ok_or_else(|| eyre::eyre!("Block not found"))?;

        println!("{}", format!("Block #{}", block.header.number).bold());
        println!("  Hash:       {:?}", block.header.hash);
        println!("  Parent:     {:?}", block.header.parent_hash);
        println!("  Timestamp:  {}", block.header.timestamp);
        println!("  Gas Used:   {}", block.header.gas_used);
        println!("  Gas Limit:  {}", block.header.gas_limit);
        println!("  Txs:        {}", block.transactions.len());

        if let Some(txs) = block.transactions.as_transactions() {
            for (i, tx) in txs.iter().enumerate() {
                println!("    [{}] hash={:?}", i, tx.inner.tx_hash());
            }
        }

        Ok(())
    }

    /// List blocks in range.
    async fn cmd_blocks(&self, from: u64, to: u64) -> eyre::Result<()> {
        let node = &self.fixture.nodes[self.active_node];

        println!("{}", format!("Blocks {} to {}:", from, to).bold());

        for n in from..=to {
            let block = node
                .node
                .rpc
                .inner
                .eth_api()
                .block_by_number(BlockNumberOrTag::Number(n), false)
                .await?;

            if let Some(block) = block {
                println!(
                    "  #{}: {} txs, gas: {}, hash: {:.12}...",
                    n,
                    block.transactions.len(),
                    block.header.gas_used,
                    format!("{:?}", block.header.hash)
                );
            } else {
                println!("  #{}: {}", n, "not found".dimmed());
            }
        }

        Ok(())
    }

    /// Show forkchoice state.
    async fn cmd_fcs(&self) -> eyre::Result<()> {
        let node = &self.fixture.nodes[self.active_node];
        let status = node.rollup_manager_handle.status().await?;
        let fcs = &status.l2.fcs;

        println!("{}", "Forkchoice State:".bold());
        println!("  Head:");
        println!("    Number: {}", fcs.head_block_info().number);
        println!("    Hash:   {:?}", fcs.head_block_info().hash);
        println!("  Safe:");
        println!("    Number: {}", fcs.safe_block_info().number);
        println!("    Hash:   {:?}", fcs.safe_block_info().hash);
        println!("  Finalized:");
        println!("    Number: {}", fcs.finalized_block_info().number);
        println!("    Hash:   {:?}", fcs.finalized_block_info().hash);

        Ok(())
    }

    /// Execute L1 commands.
    async fn cmd_l1(&mut self, cmd: L1Command) -> eyre::Result<()> {
        match cmd {
            L1Command::Status => {
                let node = &self.fixture.nodes[self.active_node];
                let status = node.rollup_manager_handle.status().await?;

                println!("{}", "L1 Status:".bold());
                println!(
                    "  Synced:    {}",
                    if status.l1.status.is_synced() { "true".green() } else { "false".red() }
                );
                println!("  L1 Head:   #{}", status.l1.latest);
                println!("  L1 Final:  #{}", status.l1.finalized);
            }
            L1Command::Sync => {
                self.fixture.l1().sync().await?;
                println!("{}", "L1 synced event sent".green());
            }
            L1Command::Block(n) => {
                self.fixture.l1().new_block(n).await?;
                println!("{}", format!("L1 block {} notification sent", n).green());
            }
            L1Command::Message(json) => {
                // Parse JSON and inject L1 message
                // For now, just show that we received the command
                println!(
                    "{}",
                    format!("L1 message injection not yet implemented. JSON: {}", json).yellow()
                );
            }
            L1Command::Commit(json) => {
                println!(
                    "{}",
                    format!("Batch commit injection not yet implemented. JSON: {}", json).yellow()
                );
            }
            L1Command::Finalize(idx) => {
                println!(
                    "{}",
                    format!("Batch finalization for index {} not yet implemented", idx).yellow()
                );
            }
            L1Command::Reorg(block) => {
                self.fixture.l1().reorg_to(block).await?;
                println!("{}", format!("L1 reorg to block {} sent", block).green());
            }
        }
        Ok(())
    }

    /// Build a new block.
    async fn cmd_build(&self) -> eyre::Result<()> {
        if !self.fixture.nodes[self.active_node].is_sequencer() {
            println!("{}", "Error: build command requires sequencer node".red());
            return Ok(());
        }

        let handle = &self.fixture.nodes[self.active_node].rollup_manager_handle;

        // Check if L1 is synced
        let status = handle.status().await?;
        if !status.l1.status.is_synced() {
            println!("{}", "Error: L1 is not synced".red());
            println!(
                "{}",
                "Hint: Run 'l1 sync' to mark the mock L1 as synced before building blocks".yellow()
            );
            return Ok(());
        }

        // Trigger block building - events will be displayed through normal event stream
        handle.build_block();
        println!("{}", "Block build triggered!".green());

        Ok(())
    }

    /// Execute transaction commands.
    async fn cmd_tx(&mut self, cmd: TxCommand) -> eyre::Result<()> {
        match cmd {
            TxCommand::Pending => {
                let node = &self.fixture.nodes[self.active_node];
                let pending_txs = node.node.inner.pool.pooled_transactions();

                if pending_txs.is_empty() {
                    println!("{}", "No pending transactions".dimmed());
                } else {
                    println!("{}", format!("Pending Transactions ({}):", pending_txs.len()).bold());
                    for (i, tx) in pending_txs.iter().enumerate() {
                        let hash = tx.hash();
                        let from = tx.sender();
                        let nonce = tx.nonce();
                        let gas_price = tx.max_fee_per_gas();
                        println!(
                            "  [{}] hash={:.16}... from={:.12}... nonce={} gas_price={}",
                            i,
                            format!("{:?}", hash),
                            format!("{:?}", from),
                            nonce,
                            gas_price
                        );
                    }
                }
            }
            TxCommand::Send { to, value, from } => {
                // Get wallet info
                let mut wallet = self.fixture.wallet.lock().await;
                let chain_id = wallet.chain_id;

                // If a wallet index is specified, use that wallet from wallet_gen()
                let (signer, nonce, from_address) = if let Some(idx) = from {
                    let wallets = wallet.wallet_gen();
                    if idx >= wallets.len() {
                        println!(
                            "{}",
                            format!(
                                "Invalid wallet index {}. Valid range: 0-{}",
                                idx,
                                wallets.len() - 1
                            )
                            .red()
                        );
                        return Ok(());
                    }
                    let signer = wallets[idx].clone();
                    let address = signer.address();

                    // Get the nonce from the chain for this wallet
                    let node = &self.fixture.nodes[self.active_node];
                    let nonce = node
                        .node
                        .rpc
                        .inner
                        .eth_api()
                        .transaction_count(address, Some(BlockNumberOrTag::Latest.into()))
                        .await?
                        .to::<u64>();

                    (signer, nonce, address)
                } else {
                    // Use the default wallet
                    let signer = wallet.inner.clone();
                    let nonce = wallet.inner_nonce;
                    let address = signer.address();

                    // Update nonce for the default wallet
                    wallet.inner_nonce += 1;

                    (signer, nonce, address)
                };
                drop(wallet);

                // Build an EIP-1559 transaction
                let mut tx = TxEip1559 {
                    chain_id,
                    nonce,
                    gas_limit: 21000,
                    max_fee_per_gas: 1_000_000_000,          // 1 gwei
                    max_priority_fee_per_gas: 1_000_000_000, // 1 gwei
                    to: TxKind::Call(to),
                    value,
                    access_list: Default::default(),
                    input: Default::default(),
                };

                // Sign the transaction
                let signature = signer.sign_transaction_sync(&mut tx)?;
                let signed = tx.into_signed(signature);

                // Encode as raw bytes (EIP-2718 envelope)
                let raw_tx = alloy_primitives::Bytes::from(signed.encoded_2718());

                // Inject the transaction
                let node = &self.fixture.nodes[self.active_node];
                let tx_hash = node.node.rpc.inject_tx(raw_tx.clone()).await?;

                println!("{}", "Transaction sent!".green());
                println!("  Hash: {:?}", tx_hash);
                println!("  From: {:?}", from_address);
                println!("  To:   {:?}", to);
                println!("  Value: {} wei", value);
                println!("{}", "Note: Run 'build' to include in a block (sequencer mode)".dimmed());
            }
            TxCommand::Inject(bytes) => {
                self.fixture.inject_tx_on(self.active_node, bytes.clone()).await?;
                println!("{}", "Transaction injected".green());
            }
        }
        Ok(())
    }

    /// Execute wallet commands.
    async fn cmd_wallet(&self, cmd: WalletCommand) -> eyre::Result<()> {
        match cmd {
            WalletCommand::Info => {
                let wallet = self.fixture.wallet.lock().await;
                let address = wallet.inner.address();
                let chain_id = wallet.chain_id;
                let nonce = wallet.inner_nonce;
                drop(wallet);

                // Get balance from the node
                let node = &self.fixture.nodes[self.active_node];
                let balance = node
                    .node
                    .rpc
                    .inner
                    .eth_api()
                    .balance(address, Some(BlockNumberOrTag::Latest.into()))
                    .await?;

                println!("{}", "Wallet Info:".bold());
                println!("  Address:  {:?}", address);
                println!("  Chain ID: {}", chain_id);
                println!("  Nonce:    {}", nonce);
                println!(
                    "  Balance:  {} wei ({:.6} ETH)",
                    balance,
                    balance.to::<u128>() as f64 / 1e18
                );
            }
            WalletCommand::Gen => {
                let wallet = self.fixture.wallet.lock().await;
                let wallets = wallet.wallet_gen();
                let chain_id = wallet.chain_id;
                drop(wallet);

                println!("{}", format!("Generated Wallets ({}):", wallets.len()).bold());
                println!("  Chain ID: {}", chain_id);
                println!();

                for (i, signer) in wallets.iter().enumerate() {
                    let address = signer.address();
                    // Get balance for each wallet
                    let node = &self.fixture.nodes[self.active_node];
                    let balance = node
                        .node
                        .rpc
                        .inner
                        .eth_api()
                        .balance(address, Some(BlockNumberOrTag::Latest.into()))
                        .await
                        .unwrap_or_default();

                    println!("  [{}] {:?}", format!("{}", i).cyan(), address);
                    println!(
                        "      Balance: {} wei ({:.6} ETH)",
                        balance,
                        balance.to::<u128>() as f64 / 1e18
                    );
                }
            }
        }
        Ok(())
    }

    /// Execute peer commands.
    async fn cmd_peers(&self, cmd: PeersCommand) -> eyre::Result<()> {
        let node = &self.fixture.nodes[self.active_node];
        let network_handle = node.rollup_manager_handle.get_network_handle().await?;

        match cmd {
            PeersCommand::List => {
                // Get this node's info
                let local_record = network_handle.local_node_record();
                let peer_count = network_handle.inner().num_connected_peers();

                println!("{}", "Local Node:".bold());
                println!("  Peer ID: {:?}", local_record.id);
                println!("  Enode:   {}", local_record);
                println!();

                // Get connected peers
                let peers = network_handle
                    .inner()
                    .get_all_peers()
                    .await
                    .map_err(|e| eyre::eyre!("Failed to get peers: {}", e))?;

                println!("{}", format!("Connected Peers ({}):", peer_count).bold());

                if peers.is_empty() {
                    println!("  {}", "No peers connected".dimmed());
                } else {
                    for peer in &peers {
                        println!("  {:?}", peer.remote_id);
                        println!("    Address:    {}", peer.remote_addr);
                        println!("    Client:     {}", peer.client_version);
                        println!("    Enode:      {}", peer.enode);
                        println!();
                    }
                }

                // Show other nodes in fixture for convenience
                if self.fixture.nodes.len() > 1 {
                    println!("{}", "Other Nodes in Fixture:".bold());
                    for (i, other_node) in self.fixture.nodes.iter().enumerate() {
                        if i == self.active_node {
                            continue;
                        }
                        let other_handle =
                            other_node.rollup_manager_handle.get_network_handle().await?;
                        let other_record = other_handle.local_node_record();
                        let node_type = match other_node.typ {
                            NodeType::Sequencer => "Sequencer",
                            NodeType::Follower => "Follower",
                        };
                        println!("  [{}] {} - {}", i, node_type, other_record);
                    }
                }
            }
            PeersCommand::Connect(enode_url) => {
                // Parse the enode URL
                match TrustedPeer::from_str(&enode_url) {
                    Ok(record) => {
                        let record = record.resolve().await?;
                        network_handle.inner().add_peer(record.id, record.tcp_addr());
                        println!("{}", format!("Connecting to peer: {:?}", record.id).green());
                        println!("  Address: {}", record.tcp_addr());
                        println!("{}", "Note: Use 'peers' to check connection status".dimmed());
                    }
                    Err(e) => {
                        println!("{}", format!("Invalid enode URL: {}", e).red());
                        println!("Expected format: enode://<pubkey>@<ip>:<port>");
                    }
                }
            }
        }
        Ok(())
    }

    /// Execute events commands.
    async fn cmd_events(&mut self, cmd: EventsCommand) -> eyre::Result<()> {
        let event_stream = &mut self.event_streams[self.active_node];
        match cmd {
            EventsCommand::On => {
                event_stream.enable();
                println!("{}", "Event stream enabled".green());
            }
            EventsCommand::Off => {
                event_stream.disable();
                println!("{}", "Event stream disabled".yellow());
            }
            EventsCommand::Stream(count) => {
                println!("Streaming {} events (Ctrl+C to stop)...", count);
                let mut received = 0;
                let node = &mut self.fixture.nodes[self.active_node];

                while received < count {
                    tokio::select! {
                        event = node.chain_orchestrator_rx.next() => {
                            if let Some(event) = event {
                                received += 1;
                                let formatted = event_stream.format_event(&event);
                                println!("[{}] {}", received, formatted);
                            }
                        }
                        _ = tokio::time::sleep(Duration::from_secs(30)) => {
                            println!("{}", "Timeout waiting for events".yellow());
                            break;
                        }
                    }
                }
            }
            EventsCommand::Filter(pattern) => {
                event_stream.set_filter(pattern.clone());
                if let Some(p) = pattern {
                    println!("{}", format!("Event filter set: {}", p).green());
                } else {
                    println!("{}", "Event filter cleared".yellow());
                }
            }
            EventsCommand::History(count) => {
                let history = event_stream.get_history(count);
                if history.is_empty() {
                    println!("{}", "No events in history".dimmed());
                } else {
                    println!("{}", format!("Last {} events:", history.len()).bold());
                    for (ago, event) in history {
                        let formatted = event_stream.format_event(event);
                        println!("  [{:?} ago] {}", ago, formatted);
                    }
                }
            }
        }
        Ok(())
    }

    /// Execute custom actions.
    async fn cmd_run(&mut self, cmd: RunCommand) -> eyre::Result<()> {
        match cmd {
            RunCommand::List => {
                println!("{}", "Available Actions:".bold());
                println!();

                let actions: Vec<_> = self.action_registry.list().collect();
                if actions.is_empty() {
                    println!("{}", "  No actions registered".dimmed());
                    println!();
                    println!(
                        "{}",
                        "To add actions, implement the Action trait and register in ActionRegistry"
                            .dimmed()
                    );
                } else {
                    for action in actions {
                        println!("  {}", action.name().cyan());
                        println!("    {}", action.description());
                        if let Some(usage) = action.usage() {
                            println!("    Usage: {}", usage.dimmed());
                        }
                        println!();
                    }
                }
            }
            RunCommand::Execute { name, args } => {
                if let Some(action) = self.action_registry.get(&name) {
                    println!("{}", format!("Running action: {}", action.name()).cyan().bold());
                    println!();

                    // Execute the action with mutable access to fixture
                    action.execute(&mut self.fixture, &args).await?;
                } else {
                    println!("{}", format!("Unknown action: {}", name).red());
                    println!("{}", "Use 'run list' to see available actions".dimmed());
                }
            }
        }
        Ok(())
    }

    /// Switch to a different node.
    fn cmd_switch_node(&mut self, idx: usize) -> eyre::Result<()> {
        if idx >= self.fixture.nodes.len() {
            println!(
                "{}",
                format!("Invalid node index. Valid range: 0-{}", self.fixture.nodes.len() - 1)
                    .red()
            );
        } else {
            self.active_node = idx;
            let node_type = match self.fixture.nodes[idx].typ {
                NodeType::Sequencer => "Sequencer",
                NodeType::Follower => "Follower",
            };
            println!("{}", format!("Switched to node {} ({})", idx, node_type).green());
        }
        Ok(())
    }

    /// List all nodes.
    fn cmd_list_nodes(&self) -> eyre::Result<()> {
        println!("{}", "Nodes:".bold());
        for (i, node) in self.fixture.nodes.iter().enumerate() {
            let node_type = match node.typ {
                NodeType::Sequencer => "Sequencer".cyan(),
                NodeType::Follower => "Follower".normal(),
            };
            let marker = if i == self.active_node { " *" } else { "" };
            println!("  [{}] {}{}", i, node_type, marker.green());
        }
        Ok(())
    }

    /// Show database path and access command.
    fn cmd_db(&self) -> eyre::Result<()> {
        let node = &self.fixture.nodes[self.active_node];
        let db_dir = node.node.inner.config.datadir().db();
        let db_path = db_dir.join("scroll.db");

        println!("{}", "Database Info:".bold());
        println!("  Path: {}", db_path.display());
        println!();
        println!("{}", "Access from another terminal:".underline());
        println!("  sqlite3 {}", db_path.display());
        println!();
        println!("{}", "Useful queries:".dimmed());
        println!("  .tables                       -- List all tables");
        println!("  .schema <table>               -- Show table schema");
        println!("  SELECT * FROM metadata;       -- View metadata");
        println!("  SELECT * FROM l2_block ORDER BY number DESC LIMIT 10;");

        Ok(())
    }
}
