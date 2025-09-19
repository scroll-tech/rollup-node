#!/usr/bin/env bash
set -e

geth init --datadir=/l2geth /l2geth-genesis-e2e.json

echo "Starting l2geth as follower..."
exec geth --datadir=/l2geth \
  --port 30303 --syncmode full --networkid 938471 --nodiscover \
  --http --http.addr 0.0.0.0 --http.port 8545 --http.vhosts "*" --http.corsdomain "*" --http.api "admin,eth,scroll,net,web3,debug" \
  --ws --ws.addr 0.0.0.0 --ws.port 8546 --ws.api "admin,eth,scroll,net,web3,debug" \
  --pprof --pprof.addr 0.0.0.0 --pprof.port 6060 --metrics --verbosity 5 --log.debug \
  --l1.endpoint "http://l1-node:8545" --l1.confirmations finalized --l1.sync.startblock 0 \
  --gcmode archive --cache.noprefetch --cache.snapshot=0 --snapshot=false \
  --gossip.enablebroadcasttoall \
  --nat extip:0.0.0.0
