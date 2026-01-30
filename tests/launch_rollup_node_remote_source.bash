#!/usr/bin/env bash
set -e

# Prepare signer key
echo -n "0xd510c4b7c61a604f800c4f06803b1ee14b9a63de345e53426ae50425f2dbb058" > /l2reth/sequencer-key

# Prepare node key for the remote source node to have a predictable enode URL
echo -n "53b69b913cb8c64a3ae83bf0ffb1e9e9efa80d7e1924dd076a8d2b1482f1b21b" > /l2reth/nodekey
# -> enode://849431bd98c23f8203cf475cfd8efb980d1e2af46337141cfb7dd960d4ae6f8c489846da293305705c8918fbf991e41805afc8c7231c96f3f938120b6826affc@rollup-node-remote-source:30303

export RUST_LOG=sqlx=off,scroll=trace,reth=info,rollup=trace,info

exec rollup-node node --chain /l2reth/l2reth-genesis-e2e.json --datadir=/l2reth --metrics=0.0.0.0:6060 \
  --disable-discovery \
  --network.valid_signer "0xb674ff99cca262c99d3eab5b32796a99188543da" \
  --http --http.addr=0.0.0.0 --http.port=8545 --http.corsdomain "*" --http.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev \
  --ws --ws.addr=0.0.0.0 --ws.port=8546 --ws.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev \
  --rpc.rollup-node-admin \
  --log.stdout.format log-fmt -vvv \
  --sequencer.enabled \
  --sequencer.allow-empty-blocks \
  --signer.key-file /l2reth/sequencer-key \
  --sequencer.block-time 500 \
  --sequencer.payload-building-duration 400 \
  --txpool.pending-max-count=1000 \
  --builder.gaslimit=30000000 \
  --rpc.max-connections=5000 \
  --p2p-secret-key /l2reth/nodekey \
  --engine.sync-at-startup false \
  --remote-source.enabled \
  --remote-source.url http://rollup-node-sequencer:8545 \
  --remote-source.poll-interval-ms 100 \
  --l1.url http://l1-node:8545 \
  --blob.mock
