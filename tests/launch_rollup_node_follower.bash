#!/usr/bin/env bash
set -e

exec rollup-node node --chain dev --datadir=/l2reth --metrics=0.0.0.0:6060 --network.scroll-wire --network.bridge \
  --http --http.addr=0.0.0.0 --http.port=8545 --http.corsdomain "*" --http.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev \
  --ws --ws.addr=0.0.0.0 --ws.port=8546 --ws.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev \
  --log.stdout.format log-fmt -vvv \
  --test \
  --txpool.pending-max-count=1000 \
  --builder.gaslimit=20000000 \
  --rpc.max-connections=5000 \
  --trusted-peers enode://3983278a7cab48862d9ab3187278edf376a0736a7deb55472a5650592f6922ce626a1ea7d74b77b9a679694b343f5e93ea97d5d60a9db4e4b51bb0c23a36d01b@rollup-node-sequencer:30303 \
  --consensus.algorithm=noop
