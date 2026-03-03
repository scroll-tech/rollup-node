//! Shared rendering for rollup node status outputs.

use colored::Colorize;
use rollup_node_chain_orchestrator::ChainOrchestratorStatus;

/// Print L2/L1 overview sections used by `status`.
pub(crate) fn print_status_overview(status: &ChainOrchestratorStatus) {
    let fcs = &status.l2.fcs;

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

    println!("{}", "L1:".underline());
    println!("  Head:      #{}", status.l1.latest.to_string().cyan());
    println!("  Finalized: #{}", status.l1.finalized);
    println!("  Processed: #{}", status.l1.processed);
    println!(
        "  Synced:    {}",
        if status.l1.status.is_synced() { "true".green() } else { "false".red() }
    );
}

/// Print detailed sync status used by `sync-status`.
pub(crate) fn print_sync_status(status: &ChainOrchestratorStatus) {
    println!("{}", "Sync Status:".bold());
    println!();
    println!("{}", "L1 Sync:".underline());
    println!(
        "  Status:    {}",
        if status.l1.status.is_synced() {
            "SYNCED".green()
        } else {
            format!("{:?}", status.l1.status).yellow().to_string().into()
        }
    );
    println!("  Latest:    #{}", status.l1.latest.to_string().cyan());
    println!("  Finalized: #{}", status.l1.finalized);
    println!("  Processed: #{}", status.l1.processed);
    println!();

    println!("{}", "L2 Sync:".underline());
    println!(
        "  Status:    {}",
        if status.l2.status.is_synced() {
            "SYNCED".green()
        } else {
            format!("{:?}", status.l2.status).yellow().to_string().into()
        }
    );
    println!();
    println!("{}", "Forkchoice:".underline());

    let fcs = &status.l2.fcs;
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
}

/// Print forkchoice section used by `fcs`.
pub(crate) fn print_forkchoice(status: &ChainOrchestratorStatus) {
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
}
