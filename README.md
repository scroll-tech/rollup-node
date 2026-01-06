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
├── book/                 # mdBook documentation (published to GitHub Pages)
├── crates/               # Internal library crates
│   ├── node/             # Main binary crate (the node)
│   ├── codec/
│   ├── database/
│   │   ├── db/
│   │   └── migration/
│   ├── derivation-pipeline/
│   ├── engine/
│   ├── chain-orchestrator/
│   ├── l1/
│   ├── network/
│   ├── primitives/
│   ├── providers/
│   ├── scroll-wire/
│   ├── signer/
│   ├── sequencer/
│   └── watcher/
├── sequencer-migration/  # Migration tooling from l2geth to l2reth
├── tests/                # Integration tests
├── Cargo.toml            # Workspace manifest
└── ...
```

## Crate Descriptions

- **crates/node/**: The main binary crate. This is the entry point for running the rollup node.
- **crates/codec/**: Implements encoding/decoding logic for rollup data and payloads.
- **crates/database/db/**: Database abstraction and storage logic for batches, blocks, and messages.
- **crates/database/migration/**: Database schema migrations using SeaORM.
- **crates/derivation-pipeline/**: Stateless pipeline for transforming batches into block-building payloads.
- **crates/engine/**: Core engine logic for block execution, fork choice, and payload management.
- **crates/chain-orchestrator/**: Responsible for orchestrating the L2 chain based on events from L1 and data gossiped over the P2P network.
- **crates/l1/**: Primitives and ABI bindings for L1 contracts and messages.
- **crates/network/**: P2P networking stack for node communication.
- **crates/node/**: Node manager and orchestration logic.
- **crates/primitives/**: Shared primitive types (blocks, batches, attributes, etc.).
- **crates/providers/**: Abstractions for data providers (L1, beacon, block, etc.).
- **crates/signer/**: Implements signing logic for blocks.
- **crates/scroll-wire/**: Wire protocol definitions for Scroll-specific networking.
- **crates/sequencer/**: Sequencer logic for ordering and batching transactions.
- **crates/watcher/**: Monitors L1 chain state and handles reorgs and notifications.
- **book/**: mdBook documentation published to [https://scroll-tech.github.io/rollup-node/](https://scroll-tech.github.io/rollup-node/)
- **tests/**: Integration tests for the rollup node, including E2E and sequencer migration tests
- **sequencer-migration/**: Scripts and tooling for migrating from l2geth to rollup-node (l2reth)

## Building the Project

Ensure you have [Rust](https://www.rust-lang.org/tools/install) installed.

To build the main binary:

```sh
cargo build --bin rollup-node
```

Or, from the binary crate directory:

```sh
cd crates/node
cargo build
```

## Running the Node

For comprehensive instructions on running a node, including:
- Hardware requirements
- Configuration options
- Example configurations for mainnet and sepolia
- Logging and debugging
- Troubleshooting

Please refer to the official documentation: **[https://scroll-tech.github.io/rollup-node/](https://scroll-tech.github.io/rollup-node/)**

For development and testing, you can run the node with test configuration:


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

## Documentation

- **[Official Documentation Book](https://scroll-tech.github.io/rollup-node/)** - Comprehensive guide for running follower and sequencer nodes
- **[Sequencer Migration Guide](./sequencer-migration/README.md)** - Documentation for migrating from l2geth to rollup-node

## Contributing

- Fork and clone the repository.
- Use `make pr` to ensure code quality.
- Submit pull requests with clear descriptions.
- See each crate's README or source for more details on its purpose and usage.

---

For more details, see the documentation in each crate or open an issue if you have questions!