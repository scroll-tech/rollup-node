/// Interactive REPL for debugging rollup nodes.
use crate::debug_toolkit::{
    actions::ActionRegistry,
    commands::{
        print_help, AdminCommand, BlockArg, Command, EventsCommand, L1Command, PeersCommand,
        RunCommand, TxCommand, WalletCommand,
    },
    event::stream::EventStreamState,
};
use crate::test_utils::{fixture::NodeType, TestFixture};
use alloy_consensus::{SignableTransaction, TxEip1559, TxLegacy};
use alloy_eips::{eip2718::Encodable2718, BlockNumberOrTag};
use alloy_network::{TransactionResponse, TxSignerSync};
use alloy_primitives::{address, Address, Bytes, TxKind, U256};
use alloy_provider::ProviderBuilder;
use alloy_rpc_types_eth::TransactionRequest;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{sol, SolCall};
use colored::Colorize;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use futures::StreamExt;
use reth_network::PeersInfo;
use reth_network_api::Peers;
use reth_network_peers::NodeRecord;
use reth_rpc_api::EthApiServer;
use reth_transaction_pool::TransactionPool;
use scroll_alloy_network::Scroll;
use std::{io::Write, path::PathBuf, str::FromStr, time::Duration};

// L1 contract addresses
const L1_MESSENGER_ADDRESS: Address = address!("8A791620dd6260079BF849Dc5567aDC3F2FdC318");
const L1_MESSAGE_QUEUE_ADDRESS: Address = address!("Dc64a140Aa3E981100a9becA4E685f962f0cF6C9");

// L1 contract interfaces
sol! {
    /// L1 Message Queue contract interface
    interface IL1MessageQueue {
        function nextCrossDomainMessageIndex() external view returns (uint256);
    }

    /// L1 Messenger contract interface
    interface IL1ScrollMessenger {
        function sendMessage(
            address _to,
            uint256 _value,
            bytes memory _message,
            uint256 _gasLimit
        ) external payable;
    }
}

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
    /// Path to the log file.
    log_path: Option<PathBuf>,
}

impl std::fmt::Debug for DebugRepl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DebugRepl")
            .field("running", &self.running)
            .field("active_node", &self.active_node)
            .field("event_streams", &self.event_streams)
            .field("action_registry", &"ActionRegistry { ... }")
            .field("log_path", &self.log_path)
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
            log_path: None,
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

        Self {
            fixture,
            running: false,
            active_node: 0,
            event_streams,
            action_registry,
            log_path: None,
        }
    }

    /// Set the log file path.
    pub fn set_log_path(&mut self, path: PathBuf) {
        self.log_path = Some(path);
    }

    /// Get mutable access to the action registry to register custom actions.
    pub const fn action_registry_mut(&mut self) -> &mut ActionRegistry {
        &mut self.action_registry
    }

    /// Run the REPL loop.
    pub async fn run(&mut self) -> eyre::Result<()> {
        self.running = true;

        let _guard = super::terminal::RawModeGuard::new()?;

        // Print welcome message and initial status
        // Disable raw mode temporarily so println! works correctly
        let _ = disable_raw_mode();
        println!();
        println!("{}", "Scroll Debug Toolkit".bold().cyan());
        println!("Type 'help' for available commands, 'exit' to quit.");
        println!();

        // Show initial status
        self.cmd_status().await?;

        // Re-enable raw mode for input handling
        let _ = enable_raw_mode();

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
                        // Clear current line, print event, reprint prompt with input buffer
                        print!("\r\x1b[K{}\r\n{}{}", formatted, self.get_prompt(), input_buffer);
                        let _ = stdout.flush();
                    }
                }

                // Check for keyboard input (non-blocking)
                _ = tokio::time::sleep(Duration::from_millis(50)) => {
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
            Command::SyncStatus => self.cmd_sync_status().await,
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
            Command::Logs => self.cmd_logs(),
            Command::Admin(admin_cmd) => self.cmd_admin(admin_cmd).await,
            Command::Rpc { method, params } => self.cmd_rpc(&method, params.as_deref()).await,
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

        println!("{}", format!("=== Node {} ({}) ===", self.active_node, node_type).bold());

        // Node Info
        let db_path = node.node.inner.config.datadir().db().join("scroll.db");
        let http_addr = node.node.inner.rpc_server_handle().http_local_addr();
        println!("{}", "Node:".underline());
        println!("  Database:  {}", db_path.display());
        if let Some(addr) = http_addr {
            println!("  HTTP RPC:  http://{}", addr);
        }
        crate::debug_toolkit::shared::status::print_status_overview(&status);

        Ok(())
    }

    /// Show detailed sync status (`rollupNode_status` RPC equivalent).
    async fn cmd_sync_status(&self) -> eyre::Result<()> {
        let node = &self.fixture.nodes[self.active_node];
        let status = node.rollup_manager_handle.status().await?;
        crate::debug_toolkit::shared::status::print_sync_status(&status);
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
        crate::debug_toolkit::shared::status::print_forkchoice(&status);
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
            L1Command::Reorg(block) => {
                self.fixture.l1().reorg_to(block).await?;
                println!("{}", format!("L1 reorg to block {} sent", block).green());
            }
            L1Command::Messages => {
                println!("{}", "L1 Message Queue:".bold());

                let Some(provider) = &self.fixture.l1_provider else {
                    println!(
                        "{}",
                        "No L1 provider available. Start with --l1-url to enable L1 commands."
                            .yellow()
                    );
                    return Ok(());
                };

                // Use sol! generated call type for encoding
                let call = IL1MessageQueue::nextCrossDomainMessageIndexCall {};
                let call_request = TransactionRequest::default()
                    .to(L1_MESSAGE_QUEUE_ADDRESS)
                    .input(call.abi_encode().into());

                match provider.call(call_request).await {
                    Ok(result) => {
                        // Decode uint256 from result using sol! generated return type
                        match IL1MessageQueue::nextCrossDomainMessageIndexCall::abi_decode_returns(
                            &result,
                        ) {
                            Ok(index) => {
                                println!("  Next Message Index: {}", index.to_string().green());
                            }
                            Err(e) => {
                                println!("{}", format!("Failed to decode response: {}", e).red());
                            }
                        }
                    }
                    Err(e) => {
                        println!("{}", format!("Failed to query message queue: {}", e).red());
                    }
                }
            }
            L1Command::Send { to, value } => {
                println!("{}", "L1 -> L2 Bridge Transfer:".bold());

                let Some(provider) = &self.fixture.l1_provider else {
                    println!(
                        "{}",
                        "No L1 provider available. Start with --l1-url to enable L1 commands."
                            .yellow()
                    );
                    return Ok(());
                };

                println!("  To:    {:?}", to);
                println!("  Value: {} wei", value);
                println!();

                // Use sol! generated call type for encoding
                let call = IL1ScrollMessenger::sendMessageCall {
                    _to: to,
                    _value: value,
                    _message: Bytes::new(),
                    _gasLimit: U256::from(200000u64),
                };
                let calldata = call.abi_encode();

                // Use the default private key for L1 transactions
                let private_key =
                    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
                let signer: PrivateKeySigner = private_key.parse().expect("valid private key");
                let from_address = signer.address();

                // Get chain ID, nonce from L1
                let chain_id = provider.get_chain_id().await?;
                let nonce = provider.get_transaction_count(from_address).await?;

                // Build a legacy transaction (for compatibility with local L1)
                // Use fixed 0.1 gwei gas price like the cast command
                let mut tx = TxLegacy {
                    chain_id: Some(chain_id),
                    nonce,
                    gas_price: 100_000_000, // 0.1 gwei
                    gas_limit: 200_000,
                    to: TxKind::Call(L1_MESSENGER_ADDRESS),
                    value,
                    input: calldata.into(),
                };

                // Sign the transaction
                let signature = signer.sign_transaction_sync(&mut tx)?;
                let signed = tx.into_signed(signature);
                let raw_tx = signed.encoded_2718();

                // Send the transaction and wait for receipt
                println!("{}", "Sending transaction...".dimmed());
                match provider.send_raw_transaction(&raw_tx).await {
                    Ok(pending) => {
                        let tx_hash = *pending.tx_hash();
                        println!("{}", "Transaction sent!".green());
                        println!("  Hash: {:?}", tx_hash);
                        println!("  From: {:?}", from_address);
                        println!();
                        println!("{}", "Waiting for receipt...".dimmed());
                        match pending.get_receipt().await {
                            Ok(receipt) => {
                                let status_str = if receipt.status() {
                                    "Success".green()
                                } else {
                                    "Failed".red()
                                };
                                println!("  Status: {}", status_str);
                                println!("  Block:  #{}", receipt.block_number.unwrap_or(0));
                                println!();
                                println!(
                                    "{}",
                                    "The L1 message will be included in L2 after L1 block finalization."
                                        .dimmed()
                                );
                            }
                            Err(e) => {
                                println!("{}", format!("Failed to get receipt: {}", e).yellow());
                            }
                        }
                    }
                    Err(e) => {
                        println!("{}", format!("Failed to send transaction: {}", e).red());
                    }
                }
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

                crate::debug_toolkit::shared::output::print_tx_sent(
                    &format!("{:?}", tx_hash),
                    &format!("{:?}", from_address),
                    &format!("{:?}", to),
                    value,
                    true,
                );
            }
            TxCommand::Inject(bytes) => {
                let tx_hash = self.fixture.inject_tx_on(self.active_node, bytes.clone()).await?;
                crate::debug_toolkit::shared::output::print_tx_injected(&format!("{:?}", tx_hash));
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
                match NodeRecord::from_str(&enode_url) {
                    Ok(record) => {
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

    /// Call any JSON-RPC method against the active node and pretty-print result.
    async fn cmd_rpc(&self, method: &str, params: Option<&str>) -> eyre::Result<()> {
        let node = &self.fixture.nodes[self.active_node];
        let http_addr = node
            .node
            .inner
            .rpc_server_handle()
            .http_local_addr()
            .ok_or_else(|| eyre::eyre!("HTTP RPC is not available on active node"))?;
        let rpc_url = format!("http://{}", http_addr);

        let provider: alloy_provider::RootProvider<Scroll> = ProviderBuilder::default()
            .connect(rpc_url.as_str())
            .await
            .map_err(|e| eyre::eyre!("Failed to connect to node RPC {}: {}", rpc_url, e))?;
        let pretty =
            crate::debug_toolkit::shared::rpc::raw_value(&provider, method, params).await?;
        crate::debug_toolkit::shared::output::print_pretty_json(&pretty)
    }

    /// Handle admin commands directly through the in-process rollup manager handle.
    async fn cmd_admin(&self, cmd: AdminCommand) -> eyre::Result<()> {
        let handle = &self.fixture.nodes[self.active_node].rollup_manager_handle;
        match cmd {
            AdminCommand::EnableSequencing => {
                let result = handle.enable_automatic_sequencing().await?;
                crate::debug_toolkit::shared::output::print_admin_enable_result(result);
            }
            AdminCommand::DisableSequencing => {
                let result = handle.disable_automatic_sequencing().await?;
                crate::debug_toolkit::shared::output::print_admin_disable_result(result);
            }
            AdminCommand::RevertToL1Block(block_number) => {
                crate::debug_toolkit::shared::output::print_admin_revert_start(block_number);
                let result = handle.revert_to_l1_block(block_number).await?;
                crate::debug_toolkit::shared::output::print_admin_revert_result(
                    block_number,
                    result,
                );
            }
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

    /// Show log file path and tail command.
    fn cmd_logs(&self) -> eyre::Result<()> {
        crate::debug_toolkit::shared::output::print_log_file(&self.log_path);
        Ok(())
    }
}
