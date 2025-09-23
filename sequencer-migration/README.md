# Scroll Sequencer Migration 

This module contains documentation and scripts for Scroll's sequencer migration from `l2geth` to rollup node (RN) aka `l2reth`.

### Risks 
We want to minimize risks and minimize service disruption. For this we need to consider following risks:
- invalid L2 blocks produced
- L2 reorg (e.g. different blocks issued at same L2 block height)
- L1 messages skipped/reverted
- general service interruption

## Migration Procedure

To instill confidence we will do many repeated transitions from `l2geth` -> `l2reth` -> `l2geth` with the time that `l2reth` sequences increasing. 

The high-level flow of the transition will look like this:
1. `l2geth` is sequencing currently
2. Turn off `l2geth` sequencing
3. Get block height of `l2geth`
4. Wait until `l2reth` has same block height
5. Turn on `l2reth` sequencing
6. Wait until `l2reth` has sequenced until block X or for some time
7. Turn off `l2reth` sequencing
8. Wait until `l2geth` has same block height
9. Turn on `l2geth` sequencingx

### Testing locally
```bash
# this test runs for ~60 seconds and starts with l2geth sequencing and expects all nodes to reach the same block at block number 120.
RUST_LOG=info,docker-compose=off cargo test --package tests --test migrate_sequencer -- docker_test_migrate_sequencer --exact --show-output

source local.env
./migrate-sequencer.sh
```

