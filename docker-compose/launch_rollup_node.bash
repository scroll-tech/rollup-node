#!/usr/bin/env bash
set -e

wait_for_l1_devnet() {
    while ! curl -s http://l1-devnet:8545 > /dev/null; do
      echo "l1-devnet not ready, retrying in 2s..."
      sleep 2
    done
}

if [ "${ENV:-}" = "dev" ]; then
  exec rollup-node node --chain dev --datadir=/l2reth  --metrics=0.0.0.0:6060 --disable-discovery \
    --http --http.addr=0.0.0.0 --http.port=8545 --http.corsdomain "*" --http.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev \
    --ws --ws.addr=0.0.0.0 --ws.port=8546 --ws.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev \
    --log.stdout.format log-fmt -vvv \
    --test \
    --sequencer.enabled --sequencer.block-time 250 --sequencer.payload-building-duration 230 --txpool.pending-max-count=1000000 --builder.gaslimit=10000000000 \
    --rpc.max-connections=5000
elif [ "${ENV:-}" = "sepolia" ]; then
  if [ "${SHADOW_FORK}" = "true" ]; then
    wait_for_l1_devnet
    URL_PARAMS="--l1.url http://l1-devnet:8545"
  else
    URL_PARAMS="--l1.url http://l1reth-rpc.sepolia.scroll.tech:8545"
  fi
  exec rollup-node node --chain scroll-sepolia --datadir=/l2reth --metrics=0.0.0.0:6060 --disable-discovery \
    --http --http.addr=0.0.0.0 --http.port=8545 --http.corsdomain "*" --http.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev \
    --ws --ws.addr=0.0.0.0 --ws.port=8546 --ws.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev \
    --log.stdout.format log-fmt -vvv $URL_PARAMS --beacon.url http://l1reth-cl.sepolia.scroll.tech:5052 --network.scroll-wire --network.bridge \
    --trusted-peers "enode://29cee709c400533ae038a875b9ca975c8abef9eade956dcf3585e940acd5c0ae916968f514bd37d1278775aad1b7db30f7032a70202a87fd7365bd8de3c9f5fc@44.242.39.33:30303,enode://ceb1636bac5cbb262e5ad5b2cd22014bdb35ffe7f58b3506970d337a63099481814a338dbcd15f2d28757151e3ecd40ba38b41350b793cd0d910ff0436654f8c@35.85.84.250:30303,enode://dd1ac5433c5c2b04ca3166f4cb726f8ff6d2da83dbc16d9b68b1ea83b7079b371eb16ef41c00441b6e85e32e33087f3b7753ea9e8b1e3f26d3e4df9208625e7f@54.148.111.168:30303"
elif [ "${ENV:-}" = "mainnet" ]; then
  if [ "${SHADOW_FORK}" = "true" ]; then
    wait_for_l1_devnet
    URL_PARAMS="--l1.url http://l1-devnet:8545"
  else
    URL_PARAMS="--l1.url http://l1geth-rpc.mainnet.scroll.tech:8545/l1"
  fi
  exec rollup-node node --chain scroll-mainnet --datadir=/l2reth --metrics=0.0.0.0:6060 --disable-discovery \
    --http --http.addr=0.0.0.0 --http.port=8545 --http.corsdomain "*" --http.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev \
    --ws --ws.addr=0.0.0.0 --ws.port=8546 --ws.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev \
    --log.stdout.format log-fmt -vvv $URL_PARAMS --beacon.url http://l1reth-cl.mainnet.scroll.tech:5052 --network.scroll-wire --network.bridge \
    --trusted-peers "enode://c6ac91f43df3d63916ac1ae411cdd5ba249d55d48a7bec7f8cd5bb351a31aba437e5a69e8a1de74d73fdfeba8af1cfe9caf9846ecd3abf60d1ffdf4925b55b23@54.186.123.248:30303,enode://fdcc807b5d1353f3a1e98b90208ce6ef1b7d446136e51eaa8ad657b55518a2f8b37655e42375d61622e6ea18f3faf9d070c9bbdf012cf5484bcbad33b7a15fb1@44.227.91.206:30303,enode://6beb5a3efbb39be73d17630b6da48e94c0ce7ec665172111463cb470197b20c12faa1fa6f835b81c28571277d1017e65c4e426cc92a46141cf69118ecf28ac03@44.237.194.52:30303,enode://7cf893d444eb8e129dca0f6485b3df579911606e7c728be4fa55fcc5f155a37c3ce07d217ccec5447798bde465ac2bdba2cb8763d107e9f3257e787579e9f27e@52.35.203.107:30303,enode://c7b2d94e95da343db6e667a01cef90376a592f2d277fbcbf6e9c9186734ed8003d01389571bd10cdbab7a6e5adfa6f0c7b55644d0db24e0b9deb4ec80f842075@54.70.236.187:30303"
fi 