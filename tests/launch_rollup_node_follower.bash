#!/usr/bin/env bash
set -e

export RUST_LOG=sqlx=off,scroll=trace,reth=trace,rollup=trace,trace

exec rollup-node node --chain /l2reth/l2reth-genesis-e2e.json --datadir=/l2reth --metrics=0.0.0.0:6060 \
  --network.scroll-wire --network.bridge --disable-discovery \
  --network.valid_signer "0xb674ff99cca262c99d3eab5b32796a99188543da" \
  --http --http.addr=0.0.0.0 --http.port=8545 --http.corsdomain "*" --http.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev \
  --ws --ws.addr=0.0.0.0 --ws.port=8546 --ws.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev \
  --log.stdout.format log-fmt -vvv \
  --txpool.pending-max-count=1000 \
  --builder.gaslimit=30000000 \
  --rpc.max-connections=5000 \
  --engine.sync-at-startup false \
  --l1.url http://l1-node:8545
