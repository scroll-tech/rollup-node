#!/bin/bash
set -e

echo "🚀 Starting L1 devnet for testing..."

# Start anvil as L1 test network
anvil \
  --host 0.0.0.0 \
  --port 8545 \
  --ws.host 0.0.0.0 \
  --ws.port 8546 \
  --chain-id ${CHAIN_ID:-31337} \
  --mnemonic "${MNEMONIC:-test test test test test test test test test test test junk}" \
  --block-time ${BLOCK_TIME:-2} \
  --accounts 10 \
  --balance 10000 \
  --gas-limit 30000000 \
  --gas-price 1000000000
