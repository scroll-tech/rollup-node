//! Shared terminal output helpers for REPL commands.

use colored::Colorize;
use std::{fmt::Display, path::PathBuf};

pub(crate) fn print_admin_enable_result(result: bool) {
    if result {
        println!("{}", "Automatic sequencing enabled".green());
    } else {
        println!("{}", "Enable sequencing returned false".yellow());
    }
}

pub(crate) fn print_admin_disable_result(result: bool) {
    if result {
        println!("{}", "Automatic sequencing disabled".yellow());
    } else {
        println!("{}", "Disable sequencing returned false".yellow());
    }
}

pub(crate) fn print_admin_revert_start(block_number: u64) {
    println!("{}", format!("Reverting to L1 block {}...", block_number).yellow());
}

pub(crate) fn print_admin_revert_result(block_number: u64, result: bool) {
    if result {
        println!("{}", format!("Reverted to L1 block {}", block_number).green());
    } else {
        println!("{}", "Revert returned false".yellow());
    }
}

pub(crate) fn print_pretty_json(value: &serde_json::Value) -> eyre::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

pub(crate) fn print_log_file(log_path: &Option<PathBuf>) {
    println!("{}", "Log File:".bold());
    if let Some(path) = log_path {
        println!("  Path: {}", path.display());
        println!();
        println!("{}", "View logs in another terminal:".underline());
        println!("  tail -f {}", path.display());
    } else {
        println!("  {}", "No log file configured (logs going to stdout)".dimmed());
    }
}

pub(crate) fn print_tx_sent(
    tx_hash: &str,
    from: &str,
    to: &str,
    value_wei: impl Display,
    include_build_hint: bool,
) {
    println!("{}", "Transaction sent!".green());
    println!("  Hash: {}", tx_hash);
    println!("  From: {}", from);
    println!("  To:   {}", to);
    println!("  Value: {} wei", value_wei);
    if include_build_hint {
        println!("{}", "Note: Run 'build' to include in a block (sequencer mode)".dimmed());
    }
}

pub(crate) fn print_tx_injected(tx_hash: &str) {
    println!("{}", "Transaction injected!".green());
    println!("  Hash: {}", tx_hash);
}
