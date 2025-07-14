#!/usr/bin/env bash
set -e

EXTRA_PARAMS=""
if [ "${FORK_BLOCK_NUMBER}" != "" ]; then
  EXTRA_PARAMS="--fork-block-number ${FORK_BLOCK_NUMBER}"
fi

if [ "${ENV:-}" = "mainnet" ]; then
  exec anvil --fork-url http://l1geth-rpc.mainnet.scroll.tech:8545/l1 --chain-id 1 --host 0.0.0.0 --block-time 12 $EXTRA_PARAMS
elif [ "${ENV:-}" = "sepolia" ]; then
  exec anvil --fork-url http://l1reth-rpc.sepolia.scroll.tech:8545 --chain-id 11155111 --host 0.0.0.0 --block-time 12 $EXTRA_PARAMS
elif [ "${ENV:-}" = "dev" ]; then
  exec anvil --chain-id 1 --host 0.0.0.0 --block-time 12
fi
