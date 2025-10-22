#!/usr/bin/env bash
set -e

# Prepare signer key
echo -n "0xd510c4b7c61a604f800c4f06803b1ee14b9a63de345e53426ae50425f2dbb058" > /l2reth/sequencer-key

# Prepare node key for the sequencer node to have a predictable enode URL
echo -n "01c0d9156e199d89814d4b18e9eb64e25de3927f3f6d27b778177f3ff6b610ad" > /l2reth/nodekey
# -> enode://e7f7e271f62bd2b697add14e6987419758c97e83b0478bd948f5f2d271495728e7edef5bd78ad65258ac910f28e86928ead0c42ee51f2a0168d8ca23ba939766@rollup-node-sequencer:30303

export RUST_LOG=sqlx=off,scroll=trace,reth=info,rollup=trace,info

exec rollup-node node --chain /l2reth/l2reth-genesis-e2e.json --datadir=/l2reth --metrics=0.0.0.0:6060 \
  --disable-discovery \
  --network.valid_signer "0xb674ff99cca262c99d3eab5b32796a99188543da" \
  --http --http.addr=0.0.0.0 --http.port=8545 --http.corsdomain "*" --http.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev \
  --ws --ws.addr=0.0.0.0 --ws.port=8546 --ws.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev \
  --rpc.rollup-node \
  --log.stdout.format log-fmt -vvv \
  --sequencer.enabled \
  --sequencer.auto-start \
  --sequencer.max-l1-messages 10 \
  --sequencer.allow-empty-blocks \
  --signer.key-file /l2reth/sequencer-key \
  --sequencer.block-time 500 \
  --sequencer.payload-building-duration 400 \
  --txpool.pending-max-count=1000 \
  --builder.gaslimit=30000000 \
  --rpc.max-connections=5000 \
  --p2p-secret-key /l2reth/nodekey \
  --engine.sync-at-startup false \
  --l1.url http://l1-node:8545 \
  --blob.mock
