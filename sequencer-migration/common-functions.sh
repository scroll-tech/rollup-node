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

# Get block number and hash for a given RPC URL
get_latest_block_info() {
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

# Get block info for a specific block number
get_block_info() {
    local rpc_url="$1"
    local block_number="$2"
    local temp_file=$(mktemp)

    if ! cast block "$block_number" --rpc-url "$rpc_url" > "$temp_file" 2>/dev/null; then
        rm -f "$temp_file"
        return 1
    fi

    local block_num=$(grep "^number" "$temp_file" | awk '{print $2}')
    local block_hash=$(grep "^hash" "$temp_file" | awk '{print $2}')

    rm -f "$temp_file"
    echo "$block_num $block_hash"
}

# Get only block number for a given RPC URL
get_block_number() {
    local rpc_url="$1"
    cast block latest --rpc-url "$rpc_url" 2>/dev/null | grep "^number" | awk '{print $2}'
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
    if cast rpc rollupNode_enableAutomaticSequencing --rpc-url "$L2RETH_RPC_URL" >/dev/null 2>&1; then
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
    if cast rpc rollupNode_disableAutomaticSequencing --rpc-url "$L2RETH_RPC_URL" >/dev/null 2>&1; then
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
                        log_success "$node_name surpassed target, now at block #$current_block (hash: $current_hash)"
                        return 0
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