#!/bin/bash

# L2GETH Block Revert Script
# Reverts l2geth to a specific block number using debug_setHead RPC
# Usage: ./revert-l2geth-to-block.sh <block_number>

set -euo pipefail

# Source common functions
source "$(dirname "$0")/common-functions.sh"

# Script-specific configuration
BLOCK_PRODUCTION_TIMEOUT=60  # 60 seconds timeout for new block production

# Global variables to track state
START_TIME=$(date +%s)
TARGET_BLOCK=""
TARGET_HASH=""

# Reset l2geth to a specific block using debug_setHead
reset_l2geth_to_block() {
    local block_number="$1"

    log_info "Resetting l2geth to block #$block_number using debug_setHead..."

    # Convert block number to hex format for debug_setHead
    local block_hex=$(printf "0x%x" "$block_number")

    if cast rpc debug_setHead --rpc-url "$L2GETH_RPC_URL" "$block_hex" >/dev/null 2>&1; then
        log_success "L2GETH reset to block #$block_number"
        return 0
    else
        log_error "Failed to reset l2geth to block #$block_number"
        return 1
    fi
}

# Wait for new block production after reset
wait_for_new_block_production() {
    local starting_block="$1"
    local expected_next_block=$((starting_block + 1))

    log_info "Waiting for l2geth to produce new blocks starting from #$expected_next_block..."

    local start_time=$(date +%s)
    while true; do
        local current_time=$(date +%s)
        local elapsed=$((current_time - start_time))

        if [[ $elapsed -gt $BLOCK_PRODUCTION_TIMEOUT ]]; then
            log_error "Timeout waiting for l2geth to produce new block (waited ${BLOCK_PRODUCTION_TIMEOUT}s)"
            return 1
        fi

        local current_block=$(get_block_number "$L2GETH_RPC_URL")
        if [[ $current_block -ge $expected_next_block ]]; then
            local block_info=$(get_latest_block_info "$L2GETH_RPC_URL")
            local block_hash=$(echo "$block_info" | awk '{print $2}')
            log_success "L2GETH produced new block #$current_block (hash: $block_hash)"
            return 0
        fi

        sleep $POLL_INTERVAL
    done
}


pre_flight_checks() {
    log_info "=== PRE-FLIGHT CHECKS ==="

    check_rpc_connectivity

    # Get current block states
    local l2geth_info=$(get_latest_block_info "$L2GETH_RPC_URL")
    local l2reth_info=$(get_latest_block_info "$L2RETH_RPC_URL")

    local current_l2geth_block=$(echo "$l2geth_info" | awk '{print $1}')
    local l2geth_hash=$(echo "$l2geth_info" | awk '{print $2}')
    local current_l2reth_block=$(echo "$l2reth_info" | awk '{print $1}')
    local l2reth_hash=$(echo "$l2reth_info" | awk '{print $2}')

    log_info "L2GETH current block: #$current_l2geth_block (hash: $l2geth_hash)"
    log_info "L2RETH current block: #$current_l2reth_block (hash: $l2reth_hash)"

    # Validate target block exists and is reachable
    if [[ $TARGET_BLOCK -gt $current_l2geth_block ]]; then
        log_error "Target block #$TARGET_BLOCK is greater than current l2geth block #$current_l2geth_block"
        log_error "Can only revert to an existing block"
        exit 1
    fi

    # Get target block info to verify it exists
    local target_info
    if ! target_info=$(get_block_info "$L2GETH_RPC_URL" "$TARGET_BLOCK"); then
        log_error "Target block #$TARGET_BLOCK not found in l2geth"
        exit 1
    fi

    TARGET_HASH=$(echo "$target_info" | awk '{print $2}')
    log_info "Target block #$TARGET_BLOCK exists (hash: $TARGET_HASH)"

    log_success "Pre-flight checks completed"
}

print_summary() {
    local end_time=$(date +%s)
    local total_time=$((end_time - START_TIME))

    log_info "=== REVERT SUMMARY ==="
    log_info "Revert completed in ${total_time}s"
    log_info "Target block: #$TARGET_BLOCK (hash: $TARGET_HASH)"

    local final_l2geth_info=$(get_latest_block_info "$L2GETH_RPC_URL")
    local final_l2geth_block=$(echo "$final_l2geth_info" | awk '{print $1}')
    local final_l2geth_hash=$(echo "$final_l2geth_info" | awk '{print $2}')
    log_info "Final L2GETH block: #$final_l2geth_block (hash: $final_l2geth_hash)"

    log_success "L2GETH revert completed successfully!"
}

main() {
    # Check arguments
    if [[ $# -ne 1 ]]; then
        echo "Usage: $0 <block_number>"
        echo "  block_number: Block number to revert l2geth to"
        exit 1
    fi

    TARGET_BLOCK="$1"

    # Validate target block is a non-negative integer
    if ! [[ "$TARGET_BLOCK" =~ ^[0-9]+$ ]]; then
        log_error "block_number must be a non-negative integer, got: $TARGET_BLOCK"
        exit 1
    fi

    log_info "Starting l2geth revert to block #$TARGET_BLOCK"

    check_env_vars
    pre_flight_checks

    # Phase 1: Disable sequencing on both nodes
    log_info "=== PHASE 1: DISABLING SEQUENCING ==="

    # Disable l2reth sequencing first (safety measure)
    disable_l2reth_sequencing

    # Disable l2geth mining
    stop_l2geth_mining

    # Phase 2: Show current state and get confirmation
    log_info "=== PHASE 2: CONFIRMATION ==="

    local l2geth_info=$(get_latest_block_info "$L2GETH_RPC_URL")
    local l2reth_info=$(get_latest_block_info "$L2RETH_RPC_URL")

    local current_l2geth_block=$(echo "$l2geth_info" | awk '{print $1}')
    local current_l2geth_hash=$(echo "$l2geth_info" | awk '{print $2}')
    local current_l2reth_block=$(echo "$l2reth_info" | awk '{print $1}')
    local current_l2reth_hash=$(echo "$l2reth_info" | awk '{print $2}')

    log_info "Current L2GETH block: #$current_l2geth_block (hash: $current_l2geth_hash)"
    log_info "Current L2RETH block: #$current_l2reth_block (hash: $current_l2reth_hash)"
    log_warning "Will revert L2GETH to block #$TARGET_BLOCK (hash: $TARGET_HASH)"

    read -p "Continue with revert? (y/N): " confirm
    if [[ "$confirm" != "y" && "$confirm" != "Y" ]]; then
        log_warning "Revert aborted by user"
        # Re-enable mining before exit
        start_l2geth_mining
        exit 0
    fi

    # Phase 3: Reset l2geth to target block
    log_info "=== PHASE 3: RESETTING L2GETH ==="
    reset_l2geth_to_block "$TARGET_BLOCK"

    # Verify the reset was successful
    local reset_info=$(get_latest_block_info "$L2GETH_RPC_URL")
    local reset_block=$(echo "$reset_info" | awk '{print $1}')
    local reset_hash=$(echo "$reset_info" | awk '{print $2}')

    if [[ $reset_block -eq $TARGET_BLOCK ]]; then
        log_success "L2GETH successfully reset to block #$reset_block (hash: $reset_hash)"
    else
        log_error "Reset verification failed: expected block #$TARGET_BLOCK, got #$reset_block"
        exit 1
    fi

    # Phase 4: Re-enable l2geth sequencing
    log_info "=== PHASE 4: ENABLING L2GETH SEQUENCING ==="
    start_l2geth_mining

    # Phase 5: Wait for new block production
    log_info "=== PHASE 5: MONITORING NEW BLOCK PRODUCTION ==="
    # TODO: combine with wait_for_block ?
    wait_for_new_block_production "$TARGET_BLOCK"

    print_summary
}

# Run main function
main "$@"