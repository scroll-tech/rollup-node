# Anvil Integration in TestFixture

This document explains how to use the integrated Anvil L1 node in the `TestFixture`.

## Overview

The `TestFixture` now supports spawning an Anvil L1 node instance programmatically. This allows tests to run against a local Ethereum L1 environment with pre-configured state, matching the setup used in docker-compose tests.

## Basic Usage

### Enable Anvil with Default Configuration

```rust
use rollup_node::test_utils::TestFixture;

#[tokio::test]
async fn test_with_anvil() -> eyre::Result<()> {
    let mut fixture = TestFixture::builder()
        .sequencer()
        .with_anvil()  // Enable anvil with defaults
        .build()
        .await?;

    // Anvil is now running
    assert!(fixture.has_anvil());
    
    // Get the endpoint URL
    if let Some(endpoint) = fixture.anvil_endpoint() {
        println!("Anvil running at: {}", endpoint);
    }

    Ok(())
}
```

### Using Default Test State

To use the same anvil state as docker-compose tests (from `tests/anvil_state.json`):

```rust
#[tokio::test]
async fn test_with_anvil_state() -> eyre::Result<()> {
    let mut fixture = TestFixture::builder()
        .sequencer()
        .with_anvil_default_state()  // Loads tests/anvil_state.json
        .build()
        .await?;

    // Anvil is now running with pre-deployed contracts
    Ok(())
}
```

### Custom Anvil Configuration

You can customize the anvil instance:

```rust
use std::path::PathBuf;

#[tokio::test]
async fn test_with_custom_anvil() -> eyre::Result<()> {
    let mut fixture = TestFixture::builder()
        .sequencer()
        .with_anvil_state(PathBuf::from("path/to/state.json"))
        .with_anvil_chain_id(1337)
        .with_anvil_block_time(2)  // 2 second block time
        .build()
        .await?;

    Ok(())
}
```

## Configuration Options

### Builder Methods

- `.with_anvil()` - Enable anvil with default settings
- `.with_anvil_default_state()` - Enable anvil and load state from `tests/anvil_state.json`
- `.with_anvil_state(path)` - Load anvil state from a custom JSON file
- `.with_anvil_chain_id(chain_id)` - Set the chain ID (default: 22222222)
- `.with_anvil_block_time(seconds)` - Set block time in seconds (default: 1)

### TestFixture Methods

- `.has_anvil()` - Check if anvil is running
- `.anvil_endpoint()` - Get the HTTP RPC endpoint URL
- `.anvil_handle()` - Get the underlying `AnvilInstance` handle

## Default Configuration

When anvil is enabled without custom settings:

- **Chain ID**: 22222222 (matches test setup)
- **Block Time**: 1 second
- **Slots per Epoch**: 4
- **Code Size Limit**: 100,000,000

## Anvil State File Format

The anvil state file should be a JSON file compatible with `anvil --load-state`. This typically includes:

- Account balances
- Deployed contracts and their code
- Contract storage state
- Block number and timestamp

Example structure:
```json
{
  "accounts": {
    "0x...": {
      "balance": "0x...",
      "nonce": "0x0",
      "code": "0x...",
      "storage": {}
    }
  }
}
```

## Integration with L1 Helpers

The anvil instance can be used with the L1 helper methods:

```rust
#[tokio::test]
async fn test_l1_interactions() -> eyre::Result<()> {
    let mut fixture = TestFixture::builder()
        .sequencer()
        .with_anvil_default_state()
        .build()
        .await?;

    // Send L1 notifications to nodes
    fixture.l1()
        .batch_commit(None, 1)
        .await?;

    Ok(())
}
```

## Complete Example

Here's a complete example test using anvil with sequencer and followers:

```rust
use rollup_node::test_utils::TestFixture;

#[tokio::test]
async fn test_full_setup_with_anvil() -> eyre::Result<()> {
    // Create fixture with anvil, sequencer, and followers
    let mut fixture = TestFixture::builder()
        .sequencer()
        .followers(2)
        .with_anvil_default_state()
        .with_anvil_block_time(1)
        .build()
        .await?;

    // Verify anvil is running
    assert!(fixture.has_anvil());
    println!("Anvil endpoint: {}", fixture.anvil_endpoint().unwrap());

    // Inject a transaction
    let tx_hash = fixture.inject_transfer().await?;

    // Build a block
    let block = fixture.build_block()
        .expect_tx(tx_hash)
        .await_block()
        .await?;

    println!("Block produced: {}", block.header.number);

    Ok(())
}
```

## Notes

- The anvil instance is automatically cleaned up when the `TestFixture` is dropped
- Each test gets its own isolated anvil instance
- The anvil state file is loaded fresh for each test run
- Anvil runs on a random available port to avoid conflicts

