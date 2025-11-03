#!/bin/bash

# Switch Sequencing to L2RETH Script
# Disables L2RETH sequencing and enables L2RETH sequencing
# Usage: ./switch-to-l2reth.sh

set -euo pipefail

# Source common functions
source "$(dirname "$0")/common-functions.sh"

# Global variables to track state
START_TIME=$(date +%s)

perform_sequencing_switch() {
    log_info "=== SWITCHING SEQUENCING TO L2RETH ==="

    # Phase 1: Disable L2GETH sequencing
    log_info "--- Phase 1: Disabling L2GETH sequencing ---"
    stop_l2geth_mining
    
    # wait for l2reth to catch up with l2geth
    log_info "-- Phase 1.5: Waiting for L2RETH to catch up with L2GETH ---"
    wait_for_l2reth_to_catch_up_with_l2geth

    # Phase 2: Enable L2GETH sequencing
    log_info "--- Phase 2: Enabling L2GETH sequencing ---"
    enable_l2reth_sequencing

    # Phase 3: Verify L2RETH is producing blocks
    log_info "--- Phase 3: Verifying L2RETH block production ---"

    # Get current block and wait for next block
    local current_block=$(get_block_number "$L2RETH_RPC_URL")
    local target_block=$((current_block + 1))

    log_info "Current L2RETH block: #$current_block, waiting for block #$target_block..."

    if wait_for_block "L2RETH" "$L2RETH_RPC_URL" "$target_block" ""; then
        log_success "L2RETH is successfully producing blocks"
    else
        log_error "L2RETH failed to produce new blocks after sequencing was enabled"
        exit 1
    fi

    log_info "Verifying L2GETH is following blocks..."

    if wait_for_l2geth_to_catch_up_with_l2reth; then
        log_success "L2GETH is successfully following blocks"
    else
        log_error "L2GETH failed to follow new blocks after L2RETH sequencing was enabled"
        exit 1
    fi
}

main() {
    log_info "Starting sequencer switch: L2GETH -> L2RETH"

    check_env_vars
    perform_pre_flight_checks

    # Final confirmation
    read -p "Proceed with switching sequencing to L2RETH? (y/N): " confirm
    if [[ "$confirm" != "y" && "$confirm" != "Y" ]]; then
        log_warning "Sequencing switch cancelled by user"
        exit 0
    fi

    perform_sequencing_switch
}

# Run main function
main "$@"