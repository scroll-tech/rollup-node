#!/usr/bin/env bash
set -e

exec rollup-node node --chain dev --datadir=/l2reth --metrics=0.0.0.0:6060 --network.scroll-wire --network.bridge \
  --http --http.addr=0.0.0.0 --http.port=8545 --http.corsdomain "*" --http.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev \
  --ws --ws.addr=0.0.0.0 --ws.port=8546 --ws.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev \
  --log.stdout.format log-fmt -vvv \
  --sequencer.enabled \
  --signer.key-file /l2reth/sequencer-key.txt
  --sequencer.block-time 250 \
  --sequencer.payload-building-duration 230 \
  --txpool.pending-max-count=1000 \
  --builder.gaslimit=20000000 \
  --rpc.max-connections=5000
