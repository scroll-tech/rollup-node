# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Architecture

This is a Rust workspace for the Scroll rollup node, built on the Reth framework. The architecture follows an event-driven, actor-based pattern with clear separation of concerns and extensive use of async channels for communication.

### Core Components & Data Flow

The system follows this critical data flow pattern:

1. **L1 Watcher** (`crates/watcher/`) monitors Ethereum L1 for events
   - Watches for batch commits, finalizations, and L1 messages
   - Handles chain reorganizations and maintains unfinalized block buffer
   - Emits `L1Notification` events to other components

2. **Chain Orchestrator** (`crates/chain-orchestrator/`) orchestrates L2 chain state
   - Receives L1 notifications and manages chain progression
   - Maintains in-memory chain buffer
   - Coordinates between L1 events and L2 block production

3. **Derivation Pipeline** (`crates/derivation-pipeline/`) transforms L1 data to L2 payloads
   - Decodes batch data using scroll-codec
   - Streams `ScrollPayloadAttributesWithBatchInfo` to engine

4. **Engine Driver** (`crates/engine/`) manages block execution
   - Interfaces with Reth execution engine via Engine API
   - Handles forkchoice updates and payload building
   - Manages consolidation of L1-derived blocks

5. **Sequencer** (`crates/sequencer/`) creates new blocks
   - Builds payload attributes from L1 messages and txpool
   - Supports different L1 message inclusion strategies
   - Configurable block time and gas limits

6. **Network Layer** (`crates/scroll-wire/`) handles P2P communication
   - Custom wire protocol for Scroll-specific block propagation
   - Manages peer discovery and block gossiping
   - Built on top of Reth networking infrastructure

### Supporting Components

- **Manager** (`crates/manager/`) - Top-level orchestration and command handling
- **Signer** (`crates/signer/`) - Block signing with AWS KMS or private key support
- **Database** (`crates/database/`) - SeaORM-based data persistence layer
- **Providers** (`crates/providers/`) - Abstractions for L1, beacon, and block data
- **Primitives** (`crates/primitives/`) - Shared types and data structures
- **Codec** (`crates/codec/`) - Encoding/decoding for rollup data formats

## Development Commands

### Build and Run
```bash
# Build main binary
cargo build --bin rollup-node

# Run sequencer with test signer (bypasses signing requirements)
cargo run --bin rollup-node -- node --chain dev -d --sequencer.enabled --test --http --http.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev

# Production build
cargo build --release --bin rollup-node

# Check node help and available options
cargo run --bin rollup-node -- node --help
```

### Testing Strategy
```bash
# Run all tests (excluding docker tests)
make test

# Test specific crate
cargo test -p <crate-name>

# Run E2E tests only
cargo test -p rollup-node --test e2e

# Run specific E2E test
cargo test -p rollup-node --test e2e test_custom_genesis_block_production_and_propagation

# Run docker-based integration tests (excluded by default)
cargo test --package tests --test block_propagation_multi_clients

# Run with debug logs
RUST_LOG=debug cargo test <test-name>
```

### Linting and Code Quality
```bash
# Complete lint suite
make lint

# Individual components
cargo +nightly fmt                    # Format code
cargo +nightly clippy --workspace     # Clippy lints
dprint fmt                            # Format TOML files
codespell --skip "*.json"             # Spell check
zepter run check                      # Feature uniformity check
cargo +nightly udeps                  # Unused dependencies check
```

### Pre-commit Workflow
```bash
# Complete pre-commit checks
make pr  # Runs lint + test + docs
```

## Critical Implementation Details

### Database Configuration
- **Default**: SQLite with path `{datadir}/scroll.db?mode=rwc`
- **Schema**: SeaORM migrations in `crates/database/migration/`
- **Key Tables**: batch_commit, block_data, l1_message, l2_block, metadata

### Sequencer Configuration
- **Signer Required**: Unless `--test` flag bypasses requirement

## Testing Patterns & Utilities

### E2E Test Setup
```rust
use rollup_node::test_utils::{
    setup_engine, 
    default_test_scroll_rollup_node_config,
    default_sequencer_test_scroll_rollup_node_config
};

// Standard E2E test pattern
let (nodes, _tasks, _wallet) = setup_engine(
    node_config, 
    node_count, 
    chain_spec, 
    enable_discovery, 
    enable_sequencer
).await?;
```

### Test Configuration Patterns
```rust
// Test with memory database
DatabaseArgs { path: Some(PathBuf::from("sqlite::memory:")) }

// Test with mock blob source
BeaconProviderArgs { blob_source: BlobSource::Mock, ..Default::default() }

// Bypass signer in tests
test: true  // in ScrollRollupNodeConfig
```

### Mock Providers
- **MockL1Provider**: For testing L1 interactions
- **MockL2Provider**: For testing L2 block data
- **DatabaseL1MessageProvider**: For L1 message testing

## Critical Gotchas & Non-Obvious Behavior

### 1. L1 Message Processing
- **Sequential Processing**: L1 messages must be processed in queue order
- **Cursor Management**: Provider maintains cursor state for message iteration
- **Gas Limit Enforcement**: Messages rejected if they exceed block gas limit

### 2. Consensus Modes
- **Noop**: No consensus validation (test mode)
- **SystemContract**: Validates against authorized signer from L1 contract
  - **PoA**: with `consensus.authorized-signer` the signer is not read from L1 and stays constant

## Debugging & Troubleshooting

### Important Log Targets
```bash
RUST_LOG=scroll::watcher=debug          # L1 watcher events
RUST_LOG=rollup_node::sequencer=debug   # Sequencer operations
RUST_LOG=scroll::engine=debug           # Engine driver events
RUST_LOG=scroll::derivation=debug       # Derivation pipeline
RUST_LOG=scroll::network=debug          # P2P networking
```

### Channel Communication Debugging
- Components communicate via `tokio::sync::mpsc` channels
- Use `tracing::trace!` to debug message flow
- Channel capacity limits defined per component

## Code Navigation Guide

### Key Entry Points
- **Main Binary**: `crates/node/src/main.rs`
- **Node Implementation**: `crates/node/src/node.rs`
- **Configuration**: `crates/node/src/args.rs`
- **Add-ons**: `crates/node/src/add_ons/`

### Finding Specific Logic
- **Batch Processing**: `crates/derivation-pipeline/src/lib.rs:252` (`derive` function)
- **L1 Message Handling**: `crates/watcher/src/lib.rs:378` (`handle_l1_messages`)
- **Block Building**: `crates/sequencer/src/lib.rs:213` (`build_payload_attributes`)
- **Engine API**: `crates/engine/src/api.rs`
- **Network Protocol**: `crates/scroll-wire/src/`

## Dependencies & Framework Integration

### Core Dependencies
- **Reth Framework**: scroll-tech fork with Scroll-specific modifications
- **Alloy**: Ethereum primitives and RPC types
- **SeaORM**: Database operations with automated migrations
- **Tokio**: Async runtime with extensive use of channels and streams
- **scroll-alloy**: Custom consensus and RPC types for Scroll

### Contributing Guidelines
- Follow existing error handling patterns using `thiserror` and `eyre`
- Use `tracing` for structured logging with appropriate targets
- Implement comprehensive tests with appropriate mock objects
- Document public APIs with examples where possible
