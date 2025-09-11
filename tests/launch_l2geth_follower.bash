#!/usr/bin/env bash
set -e

geth init --datadir=/l2geth /l2geth-genesis-e2e.json

# Create config.toml with static nodes instead of bootnodes
echo '[Node.P2P]' > /l2geth/config.toml
echo 'StaticNodes = ["enode://8fc4f6dfd0a2ebf56560d0b0ef5e60ad7bcb01e13f929eae53a4c77086d9c1e74eb8b8c8945035d25c6287afdd871f0d41b3fd7e189697decd0f13538d1ac620@l2geth-sequencer:30303","enode://e7f7e271f62bd2b697add14e6987419758c97e83b0478bd948f5f2d271495728e7edef5bd78ad65258ac910f28e86928ead0c42ee51f2a0168d8ca23ba939766@rollup-node-sequencer:30303"]' >> /l2geth/config.toml

echo "Starting l2geth as follower..."
exec geth --datadir=/l2geth \
  --config /l2geth/config.toml \
  --port 30303 --syncmode full --networkid 938471 --nodiscover \
  --http --http.addr 0.0.0.0 --http.port 8545 --http.vhosts "*" --http.corsdomain "*" --http.api "eth,scroll,net,web3,debug" \
  --ws --ws.addr 0.0.0.0 --ws.port 8546 --ws.api "eth,scroll,net,web3,debug" \
  --pprof --pprof.addr 0.0.0.0 --pprof.port 6060 --metrics --verbosity 5 --log.debug \
  --l1.endpoint "http://l1-node:8545" --l1.confirmations finalized --l1.sync.startblock 0 \
  --gcmode archive --cache.noprefetch --cache.snapshot=0 --snapshot=false \
  --nat extip:0.0.0.0
