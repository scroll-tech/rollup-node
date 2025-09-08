#!/usr/bin/env bash
set -e

export RUST_LOG=sqlx=off,scroll=trace,reth=trace,rollup=trace,info

exec rollup-node node --chain /l2reth/l2reth-genesis-e2e.json --datadir=/l2reth --metrics=0.0.0.0:6060 --network.scroll-wire --network.bridge \
  --http --http.addr=0.0.0.0 --http.port=8545 --http.corsdomain "*" --http.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev \
  --ws --ws.addr=0.0.0.0 --ws.port=8546 --ws.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev \
  --log.stdout.format log-fmt -vvv \
  --txpool.pending-max-count=1000 \
  --builder.gaslimit=30000000 \
  --rpc.max-connections=5000 \
  --trusted-peers enode://8fc4f6dfd0a2ebf56560d0b0ef5e60ad7bcb01e13f929eae53a4c77086d9c1e74eb8b8c8945035d25c6287afdd871f0d41b3fd7e189697decd0f13538d1ac620@l2geth-sequencer:30303,enode://e7f7e271f62bd2b697add14e6987419758c97e83b0478bd948f5f2d271495728e7edef5bd78ad65258ac910f28e86928ead0c42ee51f2a0168d8ca23ba939766@rollup-node-sequencer:30303 \
  --engine.sync-at-startup false \
  --l1.url http://l1-node:8545
