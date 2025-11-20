# L1 Multi-Mode Testing Documentation

## Overview

This document describes the test suite for Issue #420: Testing multi-mode L1 event consumption, reorg handling, and node restart scenarios.

**Related Issue**: https://github.com/scroll-tech/rollup-node/issues/420

## Test Coverage

### 1. Event Consumption in Different Sync States

The test suite covers the following scenarios as specified in the issue:

| Event            | Sync State | Test Function | Expected Outcome |
| ---------------- | ---------- | ------------- | ---------------- |
| BatchCommit      | Syncing    | `test_batch_commit_while_syncing` | No change (events only processed when finalized) |
| BatchCommit      | Synced     | `test_batch_commit_while_synced` | Updates safe head immediately |
| BatchFinalized   | Syncing    | `test_batch_finalized_while_syncing` | Triggers unprocessed BatchCommit events, updates safe and finalized heads |
| BatchFinalized   | Synced     | `test_batch_finalized_while_synced` | Updates finalized head only |
| BatchRevert      | Syncing    | `test_batch_revert_while_syncing` | No effect |
| BatchRevert      | Synced     | `test_batch_revert_while_synced` | Updates safe head to last block of previous batch |

### 2. L1 Reorg Handling

Tests for handling L1 reorgs of various events:

| Reorged Event      | Test Function | Expected Outcome |
| ------------------ | ------------- | ---------------- |
| BatchCommit        | `test_l1_reorg_batch_commit` | Updates safe head to last block of previous BatchCommit |
| BatchFinalized     | `test_l1_reorg_batch_finalized_has_no_effect` | No change (finalized events can't be reorged) |

### 3. Node Shutdown and Restart

- `test_node_restart_after_l1_reorg`: Tests node restart after L1 reorg (requires implementation)

### 4. Anvil Integration

- `test_with_anvil_l1_events`: Demonstrates using real Anvil instance for L1 interactions

## Current Status

### ✅ Completed

1. **Test structure created** - All test functions are defined with proper documentation
2. **Anvil integration** - TestFixture now supports Anvil with configurable state files
3. **Test data prepared**:
   - `tests/anvil_state.json` - Anvil initial state with deployed L1 contracts
   - `crates/node/tests/testdata/test_transactions.json` - Raw transaction data
   - `crates/node/tests/testdata/batch_0_calldata.bin` - Real batch calldata
   - `crates/node/tests/testdata/batch_1_calldata.bin` - Real batch calldata

### 🚧 Needs Implementation

1. **Node restart testing infrastructure**:
   - Persistent database across test runs
   - Ability to stop and restart ChainOrchestrator
   - Reorg detection on startup

2. **Anvil state file loading**:
   - The current `spawn_anvil` implementation logs a warning about state loading
   - Need to implement proper state loading from `tests/anvil_state.json`
   - See `crates/node/src/test_utils/fixture.rs:582-587`

3. **Enhanced L1 event helpers**:
   - Consider adding `BatchRangeReverted` event helper
   - Add more granular reorg testing utilities

4. **Real L1 contract interactions**:
   - Send transactions to Anvil L1 contracts (ScrollChain, MessageQueue)
   - Parse real contract events
   - Test end-to-end flow from contract call to rollup node event processing

## Running the Tests

### Run all L1 multi-mode tests:

```bash
cargo test --test l1_multi_mode
```

### Run a specific test:

```bash
cargo test --test l1_multi_mode test_batch_commit_while_syncing
```

### Run tests with logging:

```bash
RUST_LOG=debug cargo test --test l1_multi_mode -- --nocapture
```

### Run Anvil integration tests:

```bash
# These tests are ignored by default
cargo test --test l1_multi_mode test_with_anvil_l1_events -- --ignored
```

## Next Steps

### Phase 1: Fix Compilation Issues

Some tests may have compilation errors due to:
- Rust toolchain version mismatch (proc macro ABI issues)
- Missing trait imports

**Action**: Run `cargo clean` and rebuild with the correct toolchain version.

### Phase 2: Implement Anvil State Loading

Update `TestFixtureBuilder::spawn_anvil` to properly load state from JSON:

```rust
async fn spawn_anvil(
    state_path: Option<&std::path::Path>,
    chain_id: Option<u64>,
    block_time: Option<u64>,
) -> eyre::Result<anvil::NodeHandle> {
    let mut config = anvil::NodeConfig::default();
    
    if let Some(id) = chain_id {
        config.chain_id = Some(id);
    }
    
    if let Some(time) = block_time {
        config.block_time = Some(std::time::Duration::from_secs(time));
    }
    
    // TODO: Implement state loading
    // Research the correct Anvil API for loading state from file
    
    let (_api, handle) = anvil::spawn(config).await;
    Ok(handle)
}
```

### Phase 3: Enhance Test Infrastructure

1. **Add persistent database support**:
   ```rust
   pub fn with_persistent_db(mut self, path: impl Into<PathBuf>) -> Self {
       // Implementation needed
   }
   ```

2. **Add node restart capability**:
   ```rust
   pub async fn restart_node(&mut self, index: usize) -> eyre::Result<()> {
       // Implementation needed
   }
   ```

3. **Add more event assertions**:
   ```rust
   impl EventAssertions {
       pub async fn batch_reverted(self) -> eyre::Result<()> {
           // Implementation needed
       }
       
       pub async fn l1_reorg(self) -> eyre::Result<()> {
           // Implementation needed
       }
   }
   ```

### Phase 4: Real L1 Contract Testing

Use the `test_transactions.json` data to:

1. Send real transactions to Anvil contracts
2. Trigger actual BatchCommit/BatchFinalized events
3. Verify the rollup node processes them correctly

Example structure:

```rust
#[tokio::test]
async fn test_real_l1_batch_commit() -> eyre::Result<()> {
    let mut fixture = TestFixture::builder()
        .sequencer()
        .with_anvil_default_state()
        .with_anvil_chain_id(1337)
        .build()
        .await?;
    
    // Get provider for Anvil
    let anvil = fixture.anvil.as_ref().unwrap();
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .on_http(anvil.endpoint().parse()?);
    
    // Load and send transaction from test_transactions.json
    let tx_data = load_test_transaction("batch_commit_tx_0")?;
    let tx_hash = provider.send_raw_transaction(&tx_data).await?.tx_hash();
    
    // Wait for rollup node to process the event
    fixture.expect_event().batch_consolidated().await?;
    
    Ok(())
}
```

## Environment Setup

### Required Files

1. `tests/anvil_state.json` - ✅ Already created
2. `tests/anvil.env` - ✅ Already created
3. `crates/node/tests/testdata/test_transactions.json` - ✅ Already created
4. `crates/node/tests/testdata/batch_0_calldata.bin` - ✅ Already exists
5. `crates/node/tests/testdata/batch_1_calldata.bin` - ✅ Already exists

### Contract Addresses (from anvil.env)

Key L1 contract addresses deployed in Anvil:

- **L1_SCROLL_CHAIN_PROXY**: `0x5FC8d32690cc91D4c39d9d3abcBD16989F875707`
- **L1_MESSAGE_QUEUE_V2_PROXY**: `0xDc64a140Aa3E981100a9becA4E685f962f0cF6C9`
- **L1_SYSTEM_CONFIG_PROXY**: `0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0`

## Testing Matrix

Use this matrix to track test implementation progress:

| Category | Test | Status | Notes |
|----------|------|--------|-------|
| **Sync State** | BatchCommit - Syncing | ✅ | Implemented |
| | BatchCommit - Synced | ✅ | Implemented |
| | BatchFinalized - Syncing | ✅ | Implemented |
| | BatchFinalized - Synced | ✅ | Implemented |
| | BatchRevert - Syncing | ✅ | Implemented |
| | BatchRevert - Synced | ✅ | Implemented |
| **L1 Reorg** | BatchCommit Reorg | ✅ | Implemented |
| | BatchFinalized Reorg | ✅ | Implemented |
| | BatchRevert Reorg | ⬜ | TODO |
| | BatchRangeRevert Reorg | ⬜ | TODO |
| **Node Restart** | Restart after reorg | 🚧 | Needs infrastructure |
| | Restart with unfinalized events | ⬜ | TODO |
| **Anvil Integration** | Real contract events | 🚧 | Partial implementation |

## Troubleshooting

### Proc Macro ABI Mismatch

If you see errors like:
```
proc macro server error: mismatched ABI
```

**Solution**:
```bash
cargo clean
cargo build --tests
```

### Missing Batch Calldata Files

If tests fail with "No such file or directory" for batch calldata:

**Solution**: Ensure you're running tests from the project root:
```bash
cd /Users/yiweichi/Scroll/rollup-node
cargo test --test l1_multi_mode
```

### Anvil Connection Issues

If Anvil tests time out:

**Solution**: Check that the Anvil instance is starting correctly and listening on the expected port.

## Contributing

When adding new tests:

1. Follow the existing test structure and naming conventions
2. Add comprehensive documentation comments
3. Update this README with new test coverage
4. Ensure tests are idempotent and don't depend on execution order
5. Use the builder pattern for complex test setup

## References

- [Issue #420](https://github.com/scroll-tech/rollup-node/issues/420) - Original issue
- [TestFixture Documentation](crates/node/src/test_utils/fixture.rs)
- [L1 Helper Functions](crates/node/src/test_utils/l1_helpers.rs)
- [Event Assertions](crates/node/src/test_utils/event_utils.rs)


