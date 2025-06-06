# Scroll Rollup Node Workspace

[![test](https://github.com/scroll-tech/rollup-node/actions/workflows/test.yaml/badge.svg)](https://github.com/scroll-tech/rollup-node/actions/workflows/test.yaml)
[![lint](https://github.com/scroll-tech/rollup-node/actions/workflows/lint.yaml/badge.svg)](https://github.com/scroll-tech/rollup-node/actions/workflows/lint.yaml)
![License](https://img.shields.io/badge/license-MIT-blue)
![Rust](https://img.shields.io/badge/rust-2021-orange)

## Overview

This repository is a modular Rust workspace for the Scroll rollup node. It is designed for extensibility, maintainability, and ease of contribution. It consists of multiple internal crates (libraries) and a main binary crate, organized for clear separation of concerns and reusability.

## Directory Structure

```
.
├── bin/
│   └── rollup/           # Main binary crate (the node)
│       ├── src/
│       └── assets/
├── crates/               # Internal library crates
│   ├── codec/
│   ├── database/
│   │   ├── db/
│   │   └── migration/
│   ├── derivation-pipeline/
│   ├── engine/
│   ├── indexer/
│   ├── l1/
│   ├── network/
│   ├── node/
│   ├── primitives/
│   ├── providers/
│   ├── scroll-wire/
│   ├── signer/
│   ├── sequencer/
│   └── watcher/
├── Cargo.toml            # Workspace manifest
└── ...
```

## Crate Descriptions

- **bin/rollup/**: The main binary crate. This is the entry point for running the rollup node.
- **crates/codec/**: Implements encoding/decoding logic for rollup data and payloads.
- **crates/database/db/**: Database abstraction and storage logic for batches, blocks, and messages.
- **crates/database/migration/**: Database schema migrations using SeaORM.
- **crates/derivation-pipeline/**: Stateless pipeline for transforming batches into block-building payloads.
- **crates/engine/**: Core engine logic for block execution, fork choice, and payload management.
- **crates/indexer/**: Indexes L1 and L2 data for efficient querying and notification.
- **crates/l1/**: Primitives and ABI bindings for L1 contracts and messages.
- **crates/network/**: P2P networking stack for node communication.
- **crates/node/**: Node manager and orchestration logic.
- **crates/primitives/**: Shared primitive types (blocks, batches, attributes, etc.).
- **crates/providers/**: Abstractions for data providers (L1, beacon, block, etc.).
- **crates/signer/**: Implements signing logic for blocks.
- **crates/scroll-wire/**: Wire protocol definitions for Scroll-specific networking.
- **crates/sequencer/**: Sequencer logic for ordering and batching transactions.
- **crates/watcher/**: Monitors L1 chain state and handles reorgs and notifications.

## Building the Project

Ensure you have [Rust](https://www.rust-lang.org/tools/install) installed.

To build the main binary:

```sh
cargo build --bin rollup-node
```

Or, from the binary crate directory:

```sh
cd bin/rollup
cargo build
```

## Running the Node

After building, run the node with:

```sh
cargo run --workspace --bin rollup-node -- [ARGS]
```

Replace `[ARGS]` with any runtime arguments you require.

## Running Tests

To run all tests across the workspace:

```sh
make test
```

To test a specific crate:

```sh
cargo test -p <crate-name>
```

## Running Lints

To run all lints across the workspace:

```sh
make lint
```

## Building a Release Binary

For optimized production builds:

```sh
cargo build --release --bin rollup-node
```

The release binary will be located at `target/release/rollup-node`.

## Running a Sequencer Node

To run a sequencer node you should build the binary in release mode using the instructions defined above.

Then, you can run the sequencer node with the following command:

```sh
./target/release/rollup-node node --chain dev --sequencer.enabled --signer.key-file /path/to/your/private.key --http --http.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev
```

**Note**: When running a sequencer, a signer key file is required unless the `--test` flag is specified. Use the `--signer.key-file` option to specify the path to your private key file. Keep your private key file secure and never commit it to version control.

This will start a dev node in sequencer mode with all rpc apis enabled. You can adjust the `--http.api` flag to include or exclude specific APIs as needed.

The chain will be configured with a genesis that funds 20 addresses derived from the mnemonic:
```
test test test test test test test test test test test junk
```


A list of sequencer specific configuration options can be seen below:

```sh
      --scroll-sequencer-enabled
          Enable the scroll block sequencer
      --scroll-block-time <SCROLL_BLOCK_TIME>
          The block time for the sequencer [default: 2000]
      --payload-building-duration <PAYLOAD_BUILDING_DURATION>
          The payload building duration for the sequencer (milliseconds) [default: 500]
      --max-l1-messages-per-block <MAX_L1_MESSAGES_PER_BLOCK>
          The max L1 messages per block for the sequencer [default: 4]
      --fee-recipient <FEE_RECIPIENT>
          The fee recipient for the sequencer [default: 0x5300000000000000000000000000000000000005]
      --signer.key-file <FILE_PATH>
          Path to the signer's private key file (required when sequencer is enabled)
```

## Contributing

- Fork and clone the repository.
- Use `make pr` to ensure code quality.
- Submit pull requests with clear descriptions.
- See each crate's README or source for more details on its purpose and usage.

---

For more details, see the documentation in each crate or open an issue if you have questions!