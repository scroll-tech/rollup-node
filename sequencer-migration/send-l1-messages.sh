#!/bin/bash

# Send L1 messages
# Usage: ./send-l1-messages.sh [count]

set -euo pipefail

# Source common functions
source "$(dirname "$0")/common-functions.sh"

# Check for required arguments
if [ $# -lt 1 ]; then
  log_info "usage: send-l1-messages.sh [count]"
  log_info "example: send-l1-messages.sh 10"
  exit 1
fi

check_script_env_vars() {
    # Check the L1 private key is provided
    if [[ -z "${L1_DEPLOYER_PRIVATE_KEY:-}" ]]; then
        log_error "L1_DEPLOYER_PRIVATE_KEY environment variable is required"
        exit 1
    fi

    # Check the L1 RPC URL is provided
    if [[ -z "${SCROLL_L1_DEPLOYMENT_RPC:-}" ]]; then
        log_error "SCROLL_L1_DEPLOYMENT_RPC environment variable is required"
        exit 1
    fi

    # Check the L1 message queue proxy address is provided
    if [[ -z "${L1_MESSAGE_QUEUE_V2_PROXY_ADDR:-}" ]]; then
        log_error "L1_MESSAGE_QUEUE_V2_PROXY_ADDR environment variable is required"
        exit 1
    fi

    # Check the L1 enforced tx gateway proxy address is provided
    if [[ -z "${L1_ENFORCED_TX_GATEWAY_PROXY_ADDR:-}" ]]; then
        log_error "L1_ENFORCED_TX_GATEWAY_PROXY_ADDR environment variable is required"
        exit 1
    fi

    # Check the L1 scroll messenger proxy address is provided
    if [[ -z "${L1_SCROLL_MESSENGER_PROXY_ADDR:-}" ]]; then
        log_error "L1_SCROLL_MESSENGER_PROXY_ADDR environment variable is required"
        exit 1
    fi
}

main() {
    log_info "=== SENDING L1 MESSAGES ==="

    check_script_env_vars

    address=$(cast wallet address "$L1_DEPLOYER_PRIVATE_KEY")
    start_nonce=$(cast nonce --rpc-url "$SCROLL_L1_DEPLOYMENT_RPC" "$address")
    end_nonce=$(($start_nonce + $1))

    log_info "Address: $address"
    log_info "Start nonce: $start_nonce"
    log_info "End nonce: $end_nonce"

    for (( ii = $start_nonce; ii < $end_nonce; ii += 2 )); do
    log_info ""
    log_info "Sending deposit tx with nonce #$ii"
    cast send --async --rpc-url "$SCROLL_L1_DEPLOYMENT_RPC" --private-key "$L1_DEPLOYER_PRIVATE_KEY" --legacy --gas-price 0.1gwei --nonce "$ii" --gas-limit 200000 --value 0.001ether "$L1_SCROLL_MESSENGER_PROXY_ADDR" "sendMessage(address _to, uint256 _value, bytes memory _message, uint256 _gasLimit)" 0x0000000000000000000000000000000000000002 0x1 0x 200000

    log_info ""
    next_queue_index=$(cast call --rpc-url "$SCROLL_L1_DEPLOYMENT_RPC" "$L1_MESSAGE_QUEUE_V2_PROXY_ADDR" "nextCrossDomainMessageIndex()(uint256)")
    log_info "Next queue index: $next_queue_index "

    log_info ""
    log_info "Sending enforced tx with nonce #$(( $ii + 1 ))"
    cast send --async --rpc-url "$SCROLL_L1_DEPLOYMENT_RPC" --private-key "$L1_DEPLOYER_PRIVATE_KEY" --legacy --gas-price 0.1gwei --nonce "$(( $ii + 1 ))" --gas-limit 200000 --value 0.001ether "$L1_ENFORCED_TX_GATEWAY_PROXY_ADDR" "sendTransaction(address _target, uint256 _value, uint256 _gasLimit, bytes calldata _data)" 0x0000000000000000000000000000000000000001 1 200000 0x

    log_info ""
    next_queue_index=$(cast call --rpc-url "$SCROLL_L1_DEPLOYMENT_RPC" "$L1_MESSAGE_QUEUE_V2_PROXY_ADDR" "nextCrossDomainMessageIndex()(uint256)")
    log_info "Next queue index:  $next_queue_index "
    done

    log_info "Done"
}

main "$@"
