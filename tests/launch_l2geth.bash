#!/usr/bin/env bash
set -e

./geth init --datadir=./l2geth ./l2geth/l2geth-genesis-e2e.json

# Create config.toml with static nodes instead of bootnodes
echo '[Node.P2P]' > ./l2geth/config.toml
echo 'StaticNodes = ["enode://3983278a7cab48862d9ab3187278edf376a0736a7deb55472a5650592f6922ce626a1ea7d74b77b9a679694b343f5e93ea97d5d60a9db4e4b51bb0c23a36d01b@127.0.0.1:30303"]' >> ./l2geth/config.toml

echo "Starting l2geth as follower..."
exec ./geth --datadir=./l2geth \
  --config ./l2geth/config.toml \
  --port 30304 --syncmode full --networkid 1337 --nodiscover \
  --http --http.addr 0.0.0.0 --http.port 8549 --http.vhosts "*" --http.corsdomain "*" --http.api "eth,scroll,net,web3,debug" \
  --ws --ws.addr 0.0.0.0 --ws.port 8548 --ws.api "eth,scroll,net,web3,debug" \
  --pprof --pprof.addr 0.0.0.0 --pprof.port 6060 --metrics --verbosity 5 --log.debug \
  --l1.endpoint "http://localhost:8544" --l1.confirmations finalized --l1.sync.startblock 0 \
  --gcmode archive --cache.noprefetch --cache.snapshot=0 --snapshot=false \
  --nat extip:0.0.0.0
