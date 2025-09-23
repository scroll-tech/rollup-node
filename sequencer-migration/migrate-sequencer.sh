#!/bin/bash

# Sequencer Migration Script
# Migrates sequencing from l2geth -> l2reth -> l2geth
# Usage: ./migrate-sequencer.sh <blocks_to_produce>

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
SYNC_TIMEOUT=5  # 5 seconds timeout for sync operations
POLL_INTERVAL=0.1   # Poll every 0.1 seconds

# Global variables to track state
START_TIME=$(date +%s)
INITIAL_L2GETH_BLOCK=""
INITIAL_L2RETH_BLOCK=""
L2GETH_STOP_BLOCK=""
L2RETH_FINAL_BLOCK=""

# Helper function to print colored output
log() {
    echo -e "${2:-$NC}[$(date '+%H:%M:%S')] $1${NC}"
}

log_info() { log "$1" "$BLUE"; }
log_success() { log "$1" "$GREEN"; }
log_warning() { log "$1" "$YELLOW"; }
log_error() { log "$1" "$RED"; }

# Check if required environment variables are set
check_env_vars() {
    log_info "Checking environment variables..."

    if [[ -z "${L2GETH_RPC_URL:-}" ]]; then
        log_error "L2GETH_RPC_URL environment variable is required"
        exit 1
    fi

    if [[ -z "${L2RETH_RPC_URL:-}" ]]; then
        log_error "L2RETH_RPC_URL environment variable is required"
        exit 1
    fi

    log_success "Environment variables configured"
    log_info "L2GETH_RPC_URL: $L2GETH_RPC_URL"
    log_info "L2RETH_RPC_URL: $L2RETH_RPC_URL"
}

# Get block number and hash for a given RPC URL
get_block_info() {
    local rpc_url="$1"
    local temp_file=$(mktemp)

    if ! cast block latest --rpc-url "$rpc_url" > "$temp_file" 2>/dev/null; then
        rm -f "$temp_file"
        return 1
    fi

    local block_number=$(grep "^number" "$temp_file" | awk '{print $2}')
    local block_hash=$(grep "^hash" "$temp_file" | awk '{print $2}')

    rm -f "$temp_file"
    echo "$block_number $block_hash"
}

# Get only block number for a given RPC URL
get_block_number() {
    local rpc_url="$1"
    cast block latest --rpc-url "$rpc_url" 2>/dev/null | grep "^number" | awk '{print $2}'
}

is_l2geth_mining() {
    local result=$(cast rpc eth_mining --rpc-url "$L2GETH_RPC_URL" 2>/dev/null | tr -d '"')
    [[ "$result" == "true" ]]
}

start_l2geth_mining() {
    log_info "Starting l2geth mining..."
    if cast rpc miner_start --rpc-url "$L2GETH_RPC_URL" >/dev/null 2>&1; then
        log_success "L2GETH mining started"
        return 0
    else
        log_error "Failed to start l2geth mining"
        return 1
    fi
}

stop_l2geth_mining() {
    log_info "Stopping l2geth mining..."
    if cast rpc miner_stop --rpc-url "$L2GETH_RPC_URL" >/dev/null 2>&1; then
        log_success "L2GETH mining stopped"
        return 0
    else
        log_error "Failed to stop l2geth mining"
        return 1
    fi
}

enable_l2reth_sequencing() {
    log_info "Enabling L2RETH automatic sequencing..."
    if cast rpc rollupNode_enableAutomaticSequencing --rpc-url "$L2RETH_RPC_URL" >/dev/null 2>&1; then
        log_success "L2RETH automatic sequencing enabled"
        return 0
    else
        log_error "Failed to enable L2RETH automatic sequencing"
        return 1
    fi
}

disable_l2reth_sequencing() {
    log_info "Disabling L2RETH automatic sequencing..."
    if cast rpc rollupNode_disableAutomaticSequencing --rpc-url "$L2RETH_RPC_URL" >/dev/null 2>&1; then
        log_success "L2RETH automatic sequencing disabled"
        return 0
    else
        log_error "Failed to disable L2RETH automatic sequencing"
        return 1
    fi
}

wait_for_block() {
    local rpc_url="$1"
    local target_block="$2"
    local node_name="$3"
    local target_hash="$4"

    log_info "Waiting for $node_name to reach block #$target_block (hash: $target_hash)..."

    local start_time=$(date +%s)
    while true; do
        local current_time=$(date +%s)
        local elapsed=$((current_time - start_time))

        if [[ $elapsed -gt $SYNC_TIMEOUT ]]; then
            log_error "Timeout waiting for $node_name to reach block #$target_block"
            return 1
        fi

        local block_info
        if block_info=$(get_block_info "$rpc_url"); then
            local current_block=$(echo "$block_info" | awk '{print $1}')
            local current_hash=$(echo "$block_info" | awk '{print $2}')

            if [[ "$current_block" -ge "$target_block" ]]; then
                if [[ "$current_block" -eq "$target_block" && "$current_hash" == "$target_hash" ]]; then
                    log_success "$node_name reached target block #$target_block (hash: $target_hash)"
                    return 0
                elif [[ "$current_block" -gt "$target_block" ]]; then
                    log_success "$node_name surpassed target, now at block #$current_block (hash: $current_hash)"
                    return 0
                else
                    log_warning "$node_name at block #$current_block but hash mismatch: expected $target_hash, got $current_hash"
                fi
            fi
        fi

        sleep $POLL_INTERVAL
    done
}

check_rpc_connectivity() {
    log_info "Checking RPC connectivity..."

    if ! get_block_info "$L2GETH_RPC_URL" >/dev/null; then
        log_error "Cannot connect to L2GETH at $L2GETH_RPC_URL"
        exit 1
    fi

    if ! get_block_info "$L2RETH_RPC_URL" >/dev/null; then
        log_error "Cannot connect to L2RETH at $L2RETH_RPC_URL"
        exit 1
    fi

    log_success "Both nodes are accessible"
}

pre_flight_checks() {
    log_info "=== PRE-FLIGHT CHECKS ==="

    check_rpc_connectivity

    # Get initial block states
    local l2geth_info=$(get_block_info "$L2GETH_RPC_URL")
    local l2reth_info=$(get_block_info "$L2RETH_RPC_URL")

    INITIAL_L2GETH_BLOCK=$(echo "$l2geth_info" | awk '{print $1}')
    local l2geth_hash=$(echo "$l2geth_info" | awk '{print $2}')
    INITIAL_L2RETH_BLOCK=$(echo "$l2reth_info" | awk '{print $1}')
    local l2reth_hash=$(echo "$l2reth_info" | awk '{print $2}')

    log_info "L2GETH current block: #$INITIAL_L2GETH_BLOCK (hash: $l2geth_hash)"
    log_info "L2RETH current block: #$INITIAL_L2RETH_BLOCK (hash: $l2reth_hash)"

    # Check if l2geth is mining
    if ! is_l2geth_mining; then
        log_error "L2GETH is not currently mining. Please start mining first."
        exit 1
    fi
    log_success "L2GETH is currently mining"

    # Verify nodes are on the same chain by comparing a recent block
    local compare_block=$((INITIAL_L2RETH_BLOCK < INITIAL_L2GETH_BLOCK ? INITIAL_L2RETH_BLOCK : INITIAL_L2GETH_BLOCK))
    if [[ $compare_block -gt 0 ]]; then
        local l2geth_compare_hash=$(cast block "$compare_block" --rpc-url "$L2GETH_RPC_URL" 2>/dev/null | grep "^hash" | awk '{print $2}')
        local l2reth_compare_hash=$(cast block "$compare_block" --rpc-url "$L2RETH_RPC_URL" 2>/dev/null | grep "^hash" | awk '{print $2}')

        if [[ "$l2geth_compare_hash" != "$l2reth_compare_hash" ]]; then
            log_error "Nodes are on different chains! Block #$compare_block hashes differ:"
            log_error "  L2GETH: $l2geth_compare_hash"
            log_error "  L2RETH: $l2reth_compare_hash"
            exit 1
        fi
        log_success "Nodes are on the same chain (verified at block #$compare_block)"
    fi

    log_success "Pre-flight checks completed"
}

print_summary() {
    local end_time=$(date +%s)
    local total_time=$((end_time - START_TIME))

    log_info "=== MIGRATION SUMMARY ==="
    log_info "Migration completed in ${total_time}s"
    log_info "Initial L2GETH block: #$INITIAL_L2GETH_BLOCK"
    log_info "Initial L2RETH block: #$INITIAL_L2RETH_BLOCK"
    log_info "L2GETH stopped at block: #$L2GETH_STOP_BLOCK"
    log_info "L2RETH final block: #$L2RETH_FINAL_BLOCK"

    local final_l2geth_info=$(get_block_info "$L2GETH_RPC_URL")
    local final_l2geth_block=$(echo "$final_l2geth_info" | awk '{print $1}')
    local final_l2geth_hash=$(echo "$final_l2geth_info" | awk '{print $2}')
    log_info "Final L2GETH block: #$final_l2geth_block (hash: $final_l2geth_hash)"

    log_success "Sequencer migration completed successfully!"
}

main() {
    # Check arguments
    if [[ $# -ne 1 ]]; then
        echo "Usage: $0 <blocks_to_produce>"
        echo "  blocks_to_produce: Number of blocks for L2RETH to produce during migration"
        exit 1
    fi

    local blocks_to_produce="$1"

    # Validate blocks_to_produce is a positive integer
    if ! [[ "$blocks_to_produce" =~ ^[1-9][0-9]*$ ]]; then
        log_error "blocks_to_produce must be a positive integer, got: $blocks_to_produce"
        exit 1
    fi

    log_info "Starting sequencer migration: L2GETH -> L2RETH -> L2GETH"
    log_info "L2RETH will produce $blocks_to_produce blocks"

    check_env_vars
    pre_flight_checks

    # Double check if user wants to proceed
    read -p "Proceed with migration? (y/N): " confirm
    if [[ "$confirm" != "y" && "$confirm" != "Y" ]]; then
        log_warning "Migration aborted by user"
        exit 0
    fi

    # Phase 1: Stop L2GETH sequencing
    log_info "=== PHASE 1: STOPPING L2GETH SEQUENCING ==="
    stop_l2geth_mining

    # Record where L2GETH stopped
    local stop_info=$(get_block_info "$L2GETH_RPC_URL")
    L2GETH_STOP_BLOCK=$(echo "$stop_info" | awk '{print $1}')
    local stop_hash=$(echo "$stop_info" | awk '{print $2}')
    log_success "L2GETH sequencing stopped at block #$L2GETH_STOP_BLOCK (hash: $stop_hash)"

    # Phase 2: Wait for L2RETH to sync
    log_info "=== PHASE 2: WAITING FOR L2RETH SYNC ==="
    wait_for_block "$L2RETH_RPC_URL" "$L2GETH_STOP_BLOCK" "L2RETH" "$stop_hash"

    # Phase 3: Enable L2RETH sequencing and wait for blocks
    log_info "=== PHASE 3: L2RETH SEQUENCING ($blocks_to_produce blocks) ==="
    enable_l2reth_sequencing

    local target_block=$((L2GETH_STOP_BLOCK + blocks_to_produce))
    log_info "Waiting for L2RETH to produce $blocks_to_produce blocks (target: #$target_block)..."

    # Monitor block production
    local current_block=$L2GETH_STOP_BLOCK
    while [[ $current_block -lt $target_block ]]; do
        sleep $POLL_INTERVAL
        local new_block=$(get_block_number "$L2RETH_RPC_URL")
        if [[ $new_block -gt $current_block ]]; then
            local block_info=$(get_block_info "$L2RETH_RPC_URL")
            local block_hash=$(echo "$block_info" | awk '{print $2}')
            log_success "L2RETH produced block #$new_block (hash: $block_hash)"
            current_block=$new_block
        fi
    done

    # Phase 4: Stop L2RETH sequencing
    log_info "=== PHASE 4: STOPPING L2RETH SEQUENCING ==="
    disable_l2reth_sequencing

    # Record final L2RETH block
    local final_info=$(get_block_info "$L2RETH_RPC_URL")
    L2RETH_FINAL_BLOCK=$(echo "$final_info" | awk '{print $1}')
    local final_hash=$(echo "$final_info" | awk '{print $2}')
    log_success "L2RETH sequencing stopped at block #$L2RETH_FINAL_BLOCK (hash: $final_hash)"

    # Phase 5: Wait for L2GETH to sync
    log_info "=== PHASE 5: WAITING FOR L2GETH SYNC ==="
    wait_for_block "$L2GETH_RPC_URL" "$L2RETH_FINAL_BLOCK" "L2GETH" "$final_hash"

    # Phase 6: Resume L2GETH sequencing
    log_info "=== PHASE 6: RESUMING L2GETH SEQUENCING ==="
    start_l2geth_mining

    # TODO: this could be done with wait for function?
    # Wait for at least one new block to confirm
    log_info "Waiting for L2GETH to produce at least one new block..."
    local confirmation_target=$((L2RETH_FINAL_BLOCK + 1))
    local start_time=$(date +%s)
    while true; do
        local current_time=$(date +%s)
        local elapsed=$((current_time - start_time))

        if [[ $elapsed -gt 60 ]]; then  # 1 minute timeout for first block
            log_error "Timeout waiting for L2GETH to produce new block"
            exit 1
        fi

        local current_block=$(get_block_number "$L2GETH_RPC_URL")
        if [[ $current_block -ge $confirmation_target ]]; then
            local confirm_info=$(get_block_info "$L2GETH_RPC_URL")
            local confirm_hash=$(echo "$confirm_info" | awk '{print $2}')
            log_success "L2GETH sequencing resumed, produced block #$current_block (hash: $confirm_hash)"
            break
        fi

        sleep $POLL_INTERVAL
    done

    print_summary
}

# Run main function
main "$@"