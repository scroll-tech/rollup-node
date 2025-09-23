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
To test locally run the test `docker_test_migrate_sequencer` and execute the `migrate-sequencer.sh` script.
```bash
# this test runs for ~60 seconds and starts with l2geth sequencing and expects all nodes to reach the same block at block number 120.
RUST_LOG=info,docker-compose=off cargo test --package tests --test migrate_sequencer -- docker_test_migrate_sequencer --exact --show-output

source local.env
./migrate-sequencer.sh <blocks_to_produce>

# if necessary, reset l2geth block height and start sequencing on l2geth.
./revert-l2geth-to-block.sh <block_number>
```

**Simulating failure case**: 
- To simulate the case where `l2reth` produces invalid blocks we can adjust to `--builder.gaslimit=40000000` in `launch_rollup_node_sequencer.bash`. This will produce a block with a too big jump of the gas limit and `l2geth` will reject it. In a simulation we can then "revert" `l2geth` to its latest block and start sequencing on `l2geth` again.
- Continuing on the above case we can fabricate a L2 reorg by simply resetting to any previous block. For all `l2geth` nodes the reorg will be shallow (ie before the invalid `l2reth` blocks) and for `l2reth` it will be deeper (ie all `l2reth` produced blocks + reset to previous block).

TODO: how to run with Docker

