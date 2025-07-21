#!/bin/bash
set -e

echo "🚀 Starting rollup-node for testing..."

# Wait for L1 node to be ready
echo "⏳ Waiting for L1 node to be ready..."
for i in {1..30}; do
  if curl -s -X POST \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' \
    ${L1_RPC_URL:-http://l1-devnet-test:8545} > /dev/null; then
    echo "✅ L1 node is ready"
    break
  fi
  echo "⏳ Attempt $i/30: L1 node not ready yet..."
  sleep 2
done

# Start rollup-node
exec rollup-node node \
  --chain dev \
  --datadir=/l2reth \
  --metrics=0.0.0.0:6060 \
  --disable-discovery \
  --http --http.addr=0.0.0.0 --http.port=8545 --http.corsdomain "*" \
  --http.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev \
  --ws --ws.addr 0.0.0.0 --ws.port 8546 \
  --ws.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev \
  --log.stdout.format log-fmt -vvv \
  --sequencer.enabled \
  --sequencer.block-time 1000 \
  --builder.deadline 900ms \
  --sequencer.payload-building-duration 800 \
  --txpool.pending-max-count=1000000 \
  --builder.gaslimit=10000000000 \
  --rpc.max-connections=5000 \
  --l1.url ${L1_RPC_URL:-http://l1-devnet-test:8545}
