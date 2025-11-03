#!/bin/bash

# Switch Sequencing to L2GETH Script
# Disables L2RETH sequencing and enables L2GETH sequencing
# Usage: ./switch-to-l2geth.sh

set -euo pipefail

# Source common functions
source "$(dirname "$0")/common-functions.sh"

# Global variables to track state
START_TIME=$(date +%s)

perform_sequencing_switch() {
    log_info "=== SWITCHING SEQUENCING TO L2GETH ==="

    # Phase 1: Disable L2RETH sequencing
    log_info "--- Phase 1: Disabling L2RETH sequencing ---"
    disable_l2reth_sequencing

    # wait for l2geth to catch up with l2reth
    log_info "-- Phase 1.5: Waiting for L2GETH to catch up with L2RETH ---"
    wait_for_l2geth_to_catch_up_with_l2reth

    # Phase 2: Enable L2GETH sequencing
    log_info "--- Phase 2: Enabling L2GETH sequencing ---"
    start_l2geth_mining

    # Phase 3: Verify L2GETH is producing blocks
    log_info "--- Phase 3: Verifying L2GETH block production ---"

    # Get current block and wait for next block
    local current_block=$(get_block_number "$L2GETH_RPC_URL")
    local target_block=$((current_block + 1))

    log_info "Current L2GETH block: #$current_block, waiting for block #$target_block..."

    if wait_for_block "L2GETH" "$L2GETH_RPC_URL" "$target_block" ""; then
        log_success "L2GETH is successfully producing blocks"
    else
        log_error "L2GETH failed to produce new blocks after sequencing was enabled"
        exit 1
    fi

    log_info "Verifying L2RETH is following blocks..."

    if wait_for_l2reth_to_catch_up_with_l2geth; then
        log_success "L2RETH is successfully following blocks"
    else
        log_error "L2RETH failed to follow new blocks after L2GETH sequencing was enabled"
        exit 1
    fi
}

main() {
    log_info "Starting sequencer switch: L2RETH -> L2GETH"

    check_env_vars
    perform_pre_flight_checks

    # Final confirmation
    read -p "Proceed with switching sequencing to L2GETH? (y/N): " confirm
    if [[ "$confirm" != "y" && "$confirm" != "Y" ]]; then
        log_warning "Sequencing switch cancelled by user"
        exit 0
    fi

    perform_sequencing_switch
}

# Run main function
main "$@"