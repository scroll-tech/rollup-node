#!/bin/bash

# Common Functions for Sequencer Migration Scripts
# This file contains shared functionality used by multiple scripts

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
SYNC_TIMEOUT=5  # 5 seconds timeout for sync operations
POLL_INTERVAL=0.1   # Poll every 0.1 seconds

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

# Get block number and hash for a given RPC URL and block identifier
# Parameters:
#   $1: rpc_url - The RPC endpoint URL
#   $2: block_identifier - Block number or "latest" (default: "latest")
get_block_info() {
    local rpc_url="$1"
    local block_identifier="${2:-latest}"
    local block_data

    if ! block_data=$(cast block "$block_identifier" --json --rpc-url "$rpc_url" 2>/dev/null); then
        return 1
    fi

    local block_number_hex=$(echo "$block_data" | jq -r '.number // empty')
    local block_hash=$(echo "$block_data" | jq -r '.hash // empty')

    if [[ -z "$block_number_hex" || -z "$block_hash" || "$block_number_hex" == "null" || "$block_hash" == "null" ]]; then
        return 1
    fi

    # Convert hex to decimal
    local block_number=$(echo "$block_number_hex" | xargs cast to-dec)

    echo "$block_number $block_hash"
}

# Get latest block info (convenience function)
get_latest_block_info() {
    get_block_info "$1" "latest"
}

# Get only block number for a given RPC URL
get_block_number() {
    local rpc_url="$1"
    local block_info

    if ! block_info=$(get_block_info "$rpc_url" "latest"); then
        return 1
    fi

    # Extract just the block number (first field)
    echo "$block_info" | awk '{print $1}'
}

# Get chain ID for a given RPC URL
get_chain_id() {
    local rpc_url="$1"

    if ! cast chain-id --rpc-url "$rpc_url" 2>/dev/null; then
        return 1
    fi
}

# Check if l2geth is mining
is_l2geth_mining() {
    local result=$(cast rpc eth_mining --rpc-url "$L2GETH_RPC_URL" 2>/dev/null | tr -d '"')
    [[ "$result" == "true" ]]
}

# Start l2geth mining
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

# Stop l2geth mining
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

# Enable l2reth automatic sequencing
enable_l2reth_sequencing() {
    log_info "Enabling L2RETH automatic sequencing..."
    if cast rpc rollupNodeAdmin_enableAutomaticSequencing --rpc-url "$L2RETH_RPC_URL" >/dev/null 2>&1; then
        log_success "L2RETH automatic sequencing enabled"
        return 0
    else
        log_error "Failed to enable L2RETH automatic sequencing"
        return 1
    fi
}

# Disable l2reth automatic sequencing
disable_l2reth_sequencing() {
    log_info "Disabling L2RETH automatic sequencing..."
    if cast rpc rollupNodeAdmin_disableAutomaticSequencing --rpc-url "$L2RETH_RPC_URL" >/dev/null 2>&1; then
        log_success "L2RETH automatic sequencing disabled"
        return 0
    else
        log_error "Failed to disable L2RETH automatic sequencing"
        return 1
    fi
}

# Wait for a specific block to be reached
# Parameters:
#   $1: node_name - Human readable name for logging
#   $2: rpc_url - RPC URL to query
#   $3: target_block - Block number to wait for
#   $4: target_hash (optional) - If provided, will verify exact hash match
wait_for_block() {
    local node_name="$1"
    local rpc_url="$2"
    local target_block="$3"
    local target_hash="$4"

    if [[ -n "$target_hash" ]]; then
        log_info "Waiting for $node_name to reach block #$target_block (hash: $target_hash)..."
    else
        log_info "Waiting for $node_name to reach block #$target_block or higher..."
    fi

    local start_time=$(date +%s)
    while true; do
        local current_time=$(date +%s)
        local elapsed=$((current_time - start_time))
        local block_info

        if [[ $elapsed -gt $SYNC_TIMEOUT ]]; then
            log_error "Timeout waiting for $node_name to reach block #$target_block"

            if block_info=$(get_latest_block_info "$rpc_url" 2>/dev/null); then
                local current_block=$(echo "$block_info" | awk '{print $1}')
                local current_hash=$(echo "$block_info" | awk '{print $2}')
                log_error "$node_name is at block $current_block (hash: $current_hash)"
            fi

            return 1
        fi

        
        if block_info=$(get_latest_block_info "$rpc_url"); then
            local current_block=$(echo "$block_info" | awk '{print $1}')
            local current_hash=$(echo "$block_info" | awk '{print $2}')

            if [[ "$current_block" -ge "$target_block" ]]; then
                if [[ -n "$target_hash" ]]; then
                    # Hash verification mode
                    if [[ "$current_block" -eq "$target_block" && "$current_hash" == "$target_hash" ]]; then
                        log_success "$node_name reached target block #$target_block (hash: $target_hash)"
                        return 0
                    elif [[ "$current_block" -gt "$target_block" ]]; then
                        # Verify the target block hash even though we surpassed it
                        local target_block_info=$(get_block_info "$rpc_url" "$target_block")
                        if [[ -n "$target_block_info" ]]; then
                            local actual_target_hash=$(echo "$target_block_info" | awk '{print $2}')
                            if [[ "$actual_target_hash" == "$target_hash" ]]; then
                                log_success "$node_name surpassed target, now at block #$current_block (hash: $current_hash), target block verified"
                                return 0
                            else
                                log_error "$node_name surpassed target but target block #$target_block hash mismatch: expected $target_hash, got $actual_target_hash"
                                return 1
                            fi
                        else
                            log_error "$node_name surpassed target but failed to verify target block #$target_block"
                            return 1
                        fi
                    else
                        log_warning "$node_name at block #$current_block but hash mismatch: expected $target_hash, got $current_hash"
                    fi
                else
                    # Block number only mode
                    log_success "$node_name reached block #$current_block (hash: $current_hash)"
                    return 0
                fi
            fi
        fi

        sleep $POLL_INTERVAL
    done
}

wait_for_l2reth_to_catch_up_with_l2geth() {
    log_info "Waiting for L2RETH to catch up with L2GETH..."

    local l2geth_info
    if ! l2geth_info=$(get_latest_block_info "$L2GETH_RPC_URL"); then
        log_error "Failed to get latest block info from L2GETH"
        exit 1
    fi

    local target_block=$(echo "$l2geth_info" | awk '{print $1}')
    local target_hash=$(echo "$l2geth_info" | awk '{print $2}')

    if wait_for_block "L2RETH" "$L2RETH_RPC_URL" "$target_block" "$target_hash"; then
        log_success "L2RETH has caught up with L2GETH at block #$target_block"
    else
        log_error "L2RETH failed to catch up with L2GETH"
        exit 1
    fi
}

wait_for_l2geth_to_catch_up_with_l2reth() {
    log_info "Waiting for L2GETH to catch up with L2RETH..."

    local l2reth_info
    if ! l2reth_info=$(get_latest_block_info "$L2RETH_RPC_URL"); then
        log_error "Failed to get latest block info from L2RETH"
        exit 1
    fi

    local target_block=$(echo "$l2reth_info" | awk '{print $1}')
    local target_hash=$(echo "$l2reth_info" | awk '{print $2}')

    if wait_for_block "L2GETH" "$L2GETH_RPC_URL" "$target_block" "$target_hash"; then
        log_success "L2GETH has caught up with L2RETH at block #$target_block"
    else
        log_error "L2GETH failed to catch up with L2RETH"
        exit 1
    fi
}

# Check RPC connectivity for both nodes
check_rpc_connectivity() {
    log_info "Checking RPC connectivity..."

    if ! get_latest_block_info "$L2GETH_RPC_URL" >/dev/null; then
        log_error "Cannot connect to L2GETH at $L2GETH_RPC_URL"
        exit 1
    fi

    if ! get_latest_block_info "$L2RETH_RPC_URL" >/dev/null; then
        log_error "Cannot connect to L2RETH at $L2RETH_RPC_URL"
        exit 1
    fi

    log_success "Both nodes are accessible"
}

# Common pre-flight checks for sequencer migration scripts
perform_pre_flight_checks() {
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

    # Verify nodes are on the same chain by comparing chain IDs
    local l2geth_chain_id=$(get_chain_id "$L2GETH_RPC_URL")
    local l2reth_chain_id=$(get_chain_id "$L2RETH_RPC_URL")

    if [[ -z "$l2geth_chain_id" || -z "$l2reth_chain_id" ]]; then
        log_error "Failed to retrieve chain IDs from one or both nodes"
        exit 1
    fi

    if [[ "$l2geth_chain_id" != "$l2reth_chain_id" ]]; then
        log_error "Nodes are on different chains! Chain IDs differ:"
        log_error "  L2GETH: $l2geth_chain_id"
        log_error "  L2RETH: $l2reth_chain_id"
        exit 1
    fi
    log_success "Nodes are on the same chain (Chain ID: $l2geth_chain_id)"

    log_success "Pre-flight checks completed"
}