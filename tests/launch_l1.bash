#!/usr/bin/env bash
set -e

# Source environment variables
source "/anvil.env"

# Start anvil in background
anvil --host 0.0.0.0 --port 8545 --chain-id 22222222 --accounts 10 --balance 10000 --code-size-limit 100000000 --load-state anvil_state.json --block-time 1 --slots-in-an-epoch 4 &
ANVIL_PID=$!

# Wait for anvil to start (with retry)
echo "Waiting for anvil to start..."
for i in {1..10}; do
  if cast rpc eth_blockNumber --rpc-url http://localhost:8545 > /dev/null 2>&1; then
    echo "anvil is ready"
    break
  fi
  sleep 1
  echo "Waiting ($i/10)..."
done

# # Check if anvil is running
# if ! cast rpc eth_blockNumber --rpc-url http://localhost:8545 > /dev/null 2>&1; then
#   echo "Error: anvil failed to start"
#   exit 1
# fi

# Set L1 system contract consensus address
# echo "Setting system contract consensus address..."
# cast rpc anvil_setStorageAt \
#   0x55B150d210356452e4E79cCb6B778b4e1B167091 \
#   0x0000000000000000000000000000000000000000000000000000000000000067 \
#   0x000000000000000000000000b674Ff99cca262c99D3eAb5B32796a99188543dA \
#   --rpc-url http://localhost:8545

# Verify that storage was set correctly
echo "Verifying storage..."
storage_value=$(cast storage "$L1_SYSTEM_CONFIG_PROXY_ADDR" 0x67 --rpc-url http://localhost:8545)
expected_value="0x000000000000000000000000$(echo "$L1_CONSENSUS_ADDRESS" | sed 's/0x//' | tr '[:upper:]' '[:lower:]')"

if [ "$storage_value" != "$expected_value" ]; then
  echo "Error: Storage verify failed"
  echo "Expected: $expected_value"
  echo "Actual: $storage_value"
  exit 1
fi

echo "anvil started and configured, PID: $ANVIL_PID"

# Keep container running
wait $ANVIL_PID
