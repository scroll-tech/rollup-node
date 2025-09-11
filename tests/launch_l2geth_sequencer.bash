#!/usr/bin/env bash
set -e

geth init --datadir=/l2geth /l2geth-genesis-e2e.json

# Prepare node key for the sequencer node to have a predictable enode URL
echo "054ad0005a8d36f2eff836902010ca75c306c6051ef22eaa82dc4a8800beb287" > /l2geth/nodekey
# -> enode://8fc4f6dfd0a2ebf56560d0b0ef5e60ad7bcb01e13f929eae53a4c77086d9c1e74eb8b8c8945035d25c6287afdd871f0d41b3fd7e189697decd0f13538d1ac620@l2geth-sequencer:30303

# Prepare keystore and password file for the sequencer signer key: generated via `geth account import sequencer-key.txt` 
L2GETH_KEYSTORE_STRING='{"address":"b674ff99cca262c99d3eab5b32796a99188543da","crypto":{"cipher":"aes-128-ctr","ciphertext":"e9e92784d60f3434fe7059ca7ec297da40b458429bb3d711eb40615fe39a0253","cipherparams":{"iv":"6d992a9d54ce4c48c16dd02ec6e40f45"},"kdf":"scrypt","kdfparams":{"dklen":32,"n":262144,"p":1,"r":8,"salt":"74526cd2b28edbc3131da62e05e5778ba81086a3b9b07d0a31d31b1320f7e5b2"},"mac":"9947fc130580750dded6a3454addebf7a5b59c7b9fd0da287e43d97c2e47cc79"},"id":"b6f7cacd-2fcb-4142-92bb-4a4dd26a71a3","version":3}'
echo "$L2GETH_KEYSTORE_STRING" > /l2geth/keystore/keystore.json
echo "test" > /l2geth/keystore/password.txt

# --config /l2geth/config.toml \
echo "Starting l2geth as sequencer..."
exec geth --datadir=/l2geth \
  --port 30303 --syncmode full --networkid 938471 --nodiscover \
  --http --http.addr 0.0.0.0 --http.port 8545 --http.vhosts "*" --http.corsdomain "*" --http.api "eth,scroll,net,web3,debug,miner" \
  --ws --ws.addr 0.0.0.0 --ws.port 8546 --ws.api "eth,scroll,net,web3,debug,miner" \
  --pprof --pprof.addr 0.0.0.0 --pprof.port 6060 --metrics --verbosity 5 --log.debug \
  --l1.endpoint "http://l1-node:8545" --l1.confirmations finalized --l1.sync.startblock 0 \
  --gcmode archive --cache.noprefetch --cache.snapshot=0 --snapshot=false \
  --nat extip:0.0.0.0 \
  --gossip.enablebroadcasttoall \
  --unlock "0xb674ff99cca262c99d3eab5b32796a99188543da" --password "/l2geth/keystore/password.txt" --allow-insecure-unlock \
  --miner.allowempty --miner.gaslimit 30000000
