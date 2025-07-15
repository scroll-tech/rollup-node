#!/usr/bin/env bash
set -e

wait_for_l1_devnet() {
    while ! curl -s http://l1-devnet:8545 > /dev/null; do
      echo "l1-devnet not ready, retrying in 2s..."
      sleep 2
    done
}

# Dev network follower node.
if [ "${ENV:-}" = "dev" ]; then
  geth init --datadir=/l2geth /genesis-dev.json
  echo "l2geth dev mode not configured yet"
  exit 1

# Scroll Sepolia follower node.
elif [ "${ENV:-}" = "sepolia" ]; then
  if [ "${SHADOW_FORK}" = "true" ]; then
    wait_for_l1_devnet
    L1_ENDPOINT="http://l1-devnet:8545"
  else
    L1_ENDPOINT="http://l1reth-rpc.sepolia.scroll.tech:8545"
  fi
  exec geth --scroll-sepolia --scroll-mpt --datadir=/l2geth \
    --port 30303 --syncmode full \
    --http --http.addr 0.0.0.0 --http.port 8545 --http.vhosts "*" --http.corsdomain "*" --http.api "eth,scroll,net,web3,debug" \
    --ws --ws.addr 0.0.0.0 --ws.port 8546 --ws.api "eth,scroll,net,web3,debug" \
    --pprof --pprof.addr 0.0.0.0 --pprof.port 6060 --metrics \
    --gcmode archive --cache.noprefetch --cache.snapshot=0 --snapshot=false \
    --l1.endpoint "$L1_ENDPOINT" --l1.confirmations finalized --l1.sync.startblock 4038000

# Scroll Mainnet follower node.
elif [ "${ENV:-}" = "mainnet" ]; then
  if [ "${SHADOW_FORK}" = "true" ]; then
    wait_for_l1_devnet
    L1_ENDPOINT="http://l1-devnet:8545"
  else
    L1_ENDPOINT="http://l1reth-rpc.mainnet.scroll.tech:8545"
  fi
  exec geth --scroll --scroll-mpt --datadir=/l2geth \
    --port 30303 --syncmode full \
    --http --http.addr 0.0.0.0 --http.port 8545 --http.vhosts "*" --http.corsdomain "*" --http.api "eth,scroll,net,web3,debug" \
    --ws --ws.addr 0.0.0.0 --ws.port 8546 --ws.api "eth,scroll,net,web3,debug" \
    --pprof --pprof.addr 0.0.0.0 --pprof.port 6060 --metrics \
    --gcmode archive --cache.noprefetch --cache.snapshot=0 --snapshot=false \
    --l1.endpoint "$L1_ENDPOINT" --l1.confirmations finalized --l1.sync.startblock 18306000
fi
