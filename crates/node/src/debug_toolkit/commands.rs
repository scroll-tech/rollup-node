//! Command parsing and execution for the debug REPL.

use alloy_primitives::{Address, Bytes, U256};
use colored::Colorize;
use std::str::FromStr;

/// A parsed REPL command.
#[derive(Debug, Clone)]
pub enum Command {
    /// Show node status.
    Status,
    /// Show block details.
    Block(BlockArg),
    /// List blocks in range.
    Blocks {
        /// Starting block number.
        from: u64,
        /// Ending block number.
        to: u64,
    },
    /// Show forkchoice state.
    Fcs,
    /// L1 commands.
    L1(L1Command),
    /// Build a new block.
    Build,
    /// Transaction commands.
    Tx(TxCommand),
    /// Wallet commands.
    Wallet(WalletCommand),
    /// Peer commands.
    Peers(PeersCommand),
    /// Event commands.
    Events(EventsCommand),
    /// Run a custom action.
    Run(RunCommand),
    /// Switch to a different node.
    Node(usize),
    /// List all nodes.
    Nodes,
    /// Show database path and access command.
    Db,
    /// Show help.
    Help,
    /// Exit the REPL.
    Exit,
    /// Unknown command.
    Unknown(String),
}

/// Run command variants.
#[derive(Debug, Clone)]
pub enum RunCommand {
    /// List available actions.
    List,
    /// Execute an action by name.
    Execute {
        /// Action name.
        name: String,
        /// Arguments to pass to the action.
        args: Vec<String>,
    },
}

/// Block argument: either a number or "latest".
#[derive(Debug, Clone)]
pub enum BlockArg {
    /// Latest block.
    Latest,
    /// Block by number.
    Number(u64),
}

/// L1-related commands.
#[derive(Debug, Clone)]
pub enum L1Command {
    /// Show L1 status.
    Status,
    /// Inject L1 synced event.
    Sync,
    /// Inject new L1 block.
    Block(u64),
    /// Inject L1 message (JSON).
    Message(String),
    /// Inject batch commit (JSON).
    Commit(String),
    /// Inject batch finalization.
    Finalize(u64),
    /// Inject L1 reorg.
    Reorg(u64),
}

/// Transaction-related commands.
#[derive(Debug, Clone)]
pub enum TxCommand {
    /// Send a transfer.
    Send {
        /// Recipient address.
        to: Address,
        /// Transfer value.
        value: U256,
        /// Wallet index to send from (from `wallet gen` list).
        from: Option<usize>,
    },
    /// List pending transactions.
    Pending,
    /// Inject raw transaction.
    Inject(Bytes),
}

/// Peer-related commands.
#[derive(Debug, Clone)]
pub enum PeersCommand {
    /// List connected peers.
    List,
    /// Connect to a peer.
    Connect(String),
}

/// Wallet-related commands.
#[derive(Debug, Clone)]
pub enum WalletCommand {
    /// Show wallet info (address, balance, nonce).
    Info,
    /// Generate and list available wallets.
    Gen,
}

/// Event-related commands.
#[derive(Debug, Clone)]
pub enum EventsCommand {
    /// Enable background event stream.
    On,
    /// Disable background event stream.
    Off,
    /// Stream next N events.
    Stream(usize),
    /// Set event filter.
    Filter(Option<String>),
    /// Show event history.
    History(usize),
}

impl Command {
    /// Parse a command from input string.
    pub fn parse(input: &str) -> Self {
        let input = input.trim();
        if input.is_empty() {
            return Self::Unknown(String::new());
        }

        let parts: Vec<&str> = input.split_whitespace().collect();
        let cmd = parts[0].to_lowercase();
        let args = &parts[1..];

        match cmd.as_str() {
            "status" => Self::Status,
            "block" => Self::parse_block(args),
            "blocks" => Self::parse_blocks(args),
            "fcs" | "forkchoice" => Self::Fcs,
            "l1" => Self::parse_l1(args),
            "build" => Self::Build,
            "tx" => Self::parse_tx(args),
            "wallet" => Self::parse_wallet(args),
            "peers" | "peer" => Self::parse_peers(args),
            "events" | "event" => Self::parse_events(args),
            "run" => Self::parse_run(args),
            "node" => Self::parse_node(args),
            "nodes" => Self::Nodes,
            "db" | "database" => Self::Db,
            "help" | "?" => Self::Help,
            "exit" | "quit" | "q" => Self::Exit,
            _ => Self::Unknown(cmd),
        }
    }

    fn parse_block(args: &[&str]) -> Self {
        let arg = args.first().copied().unwrap_or("latest");
        if arg == "latest" {
            Self::Block(BlockArg::Latest)
        } else {
            match arg.parse::<u64>() {
                Ok(n) => Self::Block(BlockArg::Number(n)),
                Err(_) => Self::Unknown(format!("block {}", arg)),
            }
        }
    }

    fn parse_blocks(args: &[&str]) -> Self {
        if args.len() < 2 {
            return Self::Unknown("blocks requires <from> <to> arguments".to_string());
        }
        match (args[0].parse::<u64>(), args[1].parse::<u64>()) {
            (Ok(from), Ok(to)) => Self::Blocks { from, to },
            _ => Self::Unknown("blocks requires numeric arguments".to_string()),
        }
    }

    fn parse_l1(args: &[&str]) -> Self {
        let subcmd = args.first().copied().unwrap_or("status");
        let subargs = if args.len() > 1 { &args[1..] } else { &[] };

        match subcmd {
            "status" => Self::L1(L1Command::Status),
            "sync" | "synced" => Self::L1(L1Command::Sync),
            "block" => {
                if let Some(n) = subargs.first().and_then(|s| s.parse::<u64>().ok()) {
                    Self::L1(L1Command::Block(n))
                } else {
                    Self::Unknown("l1 block requires a block number".to_string())
                }
            }
            "message" | "msg" => {
                if subargs.is_empty() {
                    Self::Unknown("l1 message requires JSON data".to_string())
                } else {
                    Self::L1(L1Command::Message(subargs.join(" ")))
                }
            }
            "commit" => {
                if subargs.is_empty() {
                    Self::Unknown("l1 commit requires JSON data".to_string())
                } else {
                    Self::L1(L1Command::Commit(subargs.join(" ")))
                }
            }
            "finalize" => {
                if let Some(n) = subargs.first().and_then(|s| s.parse::<u64>().ok()) {
                    Self::L1(L1Command::Finalize(n))
                } else {
                    Self::Unknown("l1 finalize requires a batch index".to_string())
                }
            }
            "reorg" => {
                if let Some(n) = subargs.first().and_then(|s| s.parse::<u64>().ok()) {
                    Self::L1(L1Command::Reorg(n))
                } else {
                    Self::Unknown("l1 reorg requires a block number".to_string())
                }
            }
            _ => Self::Unknown(format!("l1 {}", subcmd)),
        }
    }

    fn parse_tx(args: &[&str]) -> Self {
        let subcmd = args.first().copied().unwrap_or("pending");
        let subargs = if args.len() > 1 { &args[1..] } else { &[] };

        match subcmd {
            "pending" => Self::Tx(TxCommand::Pending),
            "send" => {
                if subargs.len() < 2 {
                    return Self::Unknown(
                        "tx send requires <to> <value> [wallet_index]".to_string(),
                    );
                }
                match (Address::from_str(subargs[0]), U256::from_str(subargs[1])) {
                    (Ok(to), Ok(value)) => {
                        let from = subargs.get(2).and_then(|s| s.parse::<usize>().ok());
                        Self::Tx(TxCommand::Send { to, value, from })
                    }
                    _ => Self::Unknown("tx send: invalid address or value".to_string()),
                }
            }
            "inject" => {
                if let Some(hex) = subargs.first() {
                    match Bytes::from_str(hex) {
                        Ok(bytes) => Self::Tx(TxCommand::Inject(bytes)),
                        Err(_) => Self::Unknown("tx inject: invalid hex data".to_string()),
                    }
                } else {
                    Self::Unknown("tx inject requires hex data".to_string())
                }
            }
            _ => Self::Unknown(format!("tx {}", subcmd)),
        }
    }

    fn parse_wallet(args: &[&str]) -> Self {
        let subcmd = args.first().copied().unwrap_or("info");
        match subcmd {
            "info" | "" => Self::Wallet(WalletCommand::Info),
            "gen" | "generate" => Self::Wallet(WalletCommand::Gen),
            _ => Self::Unknown("wallet command not recognized".to_string()),
        }
    }

    fn parse_peers(args: &[&str]) -> Self {
        let subcmd = args.first().copied().unwrap_or("list");
        let subargs = if args.len() > 1 { &args[1..] } else { &[] };

        match subcmd {
            "list" | "" => Self::Peers(PeersCommand::List),
            "connect" => {
                if let Some(enode) = subargs.first() {
                    Self::Peers(PeersCommand::Connect(enode.to_string()))
                } else {
                    Self::Unknown("peers connect requires enode URL".to_string())
                }
            }
            _ => Self::Unknown("peers command not recognized".to_string()),
        }
    }

    fn parse_events(args: &[&str]) -> Self {
        let subcmd = args.first().copied().unwrap_or("stream");
        let subargs = if args.len() > 1 { &args[1..] } else { &[] };

        match subcmd {
            "on" => Self::Events(EventsCommand::On),
            "off" => Self::Events(EventsCommand::Off),
            "filter" => {
                let pattern = subargs.first().map(|s| s.to_string());
                Self::Events(EventsCommand::Filter(pattern))
            }
            "history" => {
                let count = subargs.first().and_then(|s| s.parse().ok()).unwrap_or(20);
                Self::Events(EventsCommand::History(count))
            }
            _ => {
                // Try to parse as a number for stream count
                if let Ok(count) = subcmd.parse::<usize>() {
                    Self::Events(EventsCommand::Stream(count))
                } else {
                    Self::Events(EventsCommand::Stream(10))
                }
            }
        }
    }

    fn parse_node(args: &[&str]) -> Self {
        if let Some(n) = args.first().and_then(|s| s.parse::<usize>().ok()) {
            Self::Node(n)
        } else {
            Self::Unknown("node requires an index".to_string())
        }
    }

    fn parse_run(args: &[&str]) -> Self {
        if args.is_empty() || args[0] == "list" {
            Self::Run(RunCommand::List)
        } else {
            let name = args[0].to_string();
            let action_args: Vec<String> = args.iter().skip(1).map(|s| s.to_string()).collect();
            Self::Run(RunCommand::Execute { name, args: action_args })
        }
    }
}

/// Print the help message.
pub fn print_help() {
    println!("{}", "Scroll Debug Toolkit - Commands".bold());
    println!();
    println!("{}", "Status & Inspection:".underline());
    println!("  status              Show node status (head, safe, finalized, L1 state)");
    println!("  block [n|latest]    Display block details");
    println!("  blocks <from> <to>  List blocks in range");
    println!("  fcs                 Show forkchoice state");
    println!();
    println!("{}", "L1 Commands:".underline());
    println!("  l1 status           Show L1 sync state");
    println!("  l1 sync             Inject L1 synced event");
    println!("  l1 block <n>        Inject new L1 block notification");
    println!("  l1 message <json>   Inject an L1 message");
    println!("  l1 commit <json>    Inject batch commit");
    println!("  l1 finalize <idx>   Inject batch finalization");
    println!("  l1 reorg <block>    Inject L1 reorg");
    println!();
    println!("{}", "Block & Transaction:".underline());
    println!("  build               Build a new block (sequencer mode)");
    println!("  tx send <to> <val> [idx]  Send ETH transfer (idx = wallet index from gen)");
    println!("  tx pending          List pending transactions");
    println!("  tx inject <hex>     Inject raw transaction");
    println!();
    println!("{}", "Wallet:".underline());
    println!("  wallet              Show wallet address, balance, and nonce");
    println!("  wallet gen          Generate and list all available wallets");
    println!();
    println!("{}", "Network:".underline());
    println!("  peers               List connected peers and show local enode");
    println!("  peers connect <url> Connect to a peer (enode://...)");
    println!();
    println!("{}", "Events:".underline());
    println!("  events on           Enable background event stream");
    println!("  events off          Disable background event stream");
    println!("  events [count]      Stream next N events (default: 10)");
    println!("  events filter <pat> Filter events by type (e.g., Block*, L1*)");
    println!("  events history [n]  Show last N events (default: 20)");
    println!();
    println!("{}", "Custom Actions:".underline());
    println!("  run list            List available custom actions");
    println!("  run <name> [args]   Execute a custom action");
    println!();
    println!("{}", "Node Management:".underline());
    println!("  node <index>        Switch active node context");
    println!("  nodes               List all nodes in fixture");
    println!();
    println!("{}", "Database:".underline());
    println!("  db                  Show database path and access command");
    println!();
    println!("{}", "Other:".underline());
    println!("  help                Show this help message");
    println!("  exit                Exit the REPL");
}
