# Debug Toolkit

The Debug Toolkit is an interactive REPL (Read-Eval-Print Loop) for debugging, development, and hackathon scenarios. It provides a command-line interface to interact with rollup nodes, inspect chain state, inject transactions, and run custom actions.

## Getting Started

### Building

The debug toolkit is built with the `debug-toolkit` feature flag:

```bash
cargo build -p rollup-node --features debug-toolkit --release
```

### Running

Launch the debug REPL using the `scroll-debug` binary:

```bash
# Basic usage with dev chain and sequencer mode
cargo run --features debug-toolkit --bin scroll-debug -- --chain dev --sequencer

# With multiple follower nodes
cargo run --features debug-toolkit --bin scroll-debug -- --chain dev --sequencer --followers 2

# With custom block time (auto-build every 1000ms)
cargo run --features debug-toolkit --bin scroll-debug -- --chain dev --sequencer --block-time 1000

# With a real L1 endpoint
cargo run --features debug-toolkit --bin scroll-debug -- --chain dev --sequencer --l1-url https://eth.llamarpc.com
```

### CLI Options

| Option | Description |
|--------|-------------|
| `--chain <name\|path>` | Chain to use: `dev`, `scroll-sepolia`, `scroll-mainnet`, or path to genesis JSON file (default: `dev`) |
| `--sequencer` | Enable sequencer mode |
| `--followers <n>` | Number of follower nodes to create (default: 0) |
| `--block-time <ms>` | Block time in milliseconds (default: 0 = manual block building only) |
| `--allow-empty-blocks` | Allow building empty blocks (default: true) |
| `--l1-message-delay <n>` | L1 message inclusion delay in blocks (default: 0 = immediate) |
| `--l1-url <url>` | L1 RPC endpoint URL (optional, uses mock L1 if not specified) |

Run `cargo run --features debug-toolkit --bin scroll-debug -- --help` to see all available options.

## Quick Start: Multi-Node Environment with Mock L1

This walkthrough demonstrates how to spin up a complete local environment with a mock L1, one sequencer, and two follower nodes.

### Starting the Environment

```bash
cargo run --features debug-toolkit --bin scroll-debug -- \
    --chain dev \
    --sequencer \
    --followers 2
```

This creates:
- **Node 0**: Sequencer (produces blocks)
- **Node 1**: Follower (receives blocks via P2P)
- **Node 2**: Follower (receives blocks via P2P)

### Understanding Mock L1

When no `--l1-url` is specified, the toolkit uses a **mock L1**. The mock L1 starts in an "unsynced" state, which means the sequencer won't produce blocks until you explicitly sync it.

If you try to build a block before syncing L1:

```
scroll-debug [seq:0]> build
Error: L1 is not synced
Hint: Run 'l1 sync' to mark the mock L1 as synced before building blocks
```

### Step-by-Step Walkthrough

**1. Check initial status:**

```
scroll-debug [seq:0]> status
=== Node 0 (Sequencer) ===
Node:
  Database:  /tmp/.tmpXYZ/db/scroll.db
  HTTP RPC:  http://127.0.0.1:62491
L2:
  Head:      #0 (0x1234abcd...)
  Safe:      #0 (0x1234abcd...)
  Finalized: #0 (0x1234abcd...)
  Synced:    false
L1:
  Head:      #0
  Finalized: #0
  Processed: #0
  Synced:    false
```

Note that L1 `Synced` is `false`.

**2. Sync the mock L1:**

```
scroll-debug [seq:0]> l1 sync
L1 synced event sent to all nodes
```

**3. Build your first block:**

```
scroll-debug [seq:0]> build
Block build triggered!
  [EVENT] BlockSequenced { block: 1, hash: 0xabcd1234... }
```

**4. Send a transaction and build another block:**

```
scroll-debug [seq:0]> tx send 0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb2 1000000000000000000
Transaction sent!
  Hash: 0x5678...
  From: 0x1234...
  To:   0x742d...
  Value: 1000000000000000000 wei
Note: Run 'build' to include in a block

scroll-debug [seq:0]> build
Block build triggered!
  [EVENT] BlockSequenced { block: 2, hash: 0xefgh5678... }
```

**5. Check that followers received the blocks:**

```
scroll-debug [seq:0]> node 1
Switched to node 1 (Follower)

scroll-debug [fol:1]> status
=== Node 1 (Follower) ===
Node:
  Database:  /tmp/.tmpABC/db/scroll.db
  HTTP RPC:  http://127.0.0.1:62502
L2:
  Head:      #2 (0xefgh5678...)
  Safe:      #0 (0x1234abcd...)
  Finalized: #0 (0x1234abcd...)
  Synced:    false
...
```

The follower's head is at block #2, showing it received the blocks via P2P.

**6. Build multiple blocks quickly:**

```
scroll-debug [seq:0]> run build-blocks 5
Building 5 blocks (timeout: 5000ms per block)...
  Block 1 triggered, waiting... sequenced at #3
  Block 2 triggered, waiting... sequenced at #4
  Block 3 triggered, waiting... sequenced at #5
  Block 4 triggered, waiting... sequenced at #6
  Block 5 triggered, waiting... sequenced at #7
Done! Head is now at block #7
```

**7. View all nodes:**

```
scroll-debug [seq:0]> nodes
Nodes:
  [0] Sequencer *
  [1] Follower
  [2] Follower
```

## Commands

### Status & Inspection

| Command | Description |
|---------|-------------|
| `status` | Show node status (L2 head/safe/finalized, L1 state, sync status) |
| `block [n\|latest]` | Display block details |
| `blocks <from> <to>` | List blocks in range |
| `fcs` | Show forkchoice state |

**Example:**

```
scroll-debug [seq:0]> status
=== Node 0 (Sequencer) ===
L2:
  Head:      #42 (0x1234abcd...)
  Safe:      #40 (0x5678efgh...)
  Finalized: #35 (0x9abc1234...)
  Synced:    true
L1:
  Head:      #18923456
  Finalized: #18923400
  Processed: #18923450
  Synced:    true
```

### L1 Commands

These commands allow you to simulate L1 events:

| Command | Description |
|---------|-------------|
| `l1 status` | Show L1 sync state |
| `l1 sync` | Inject L1 synced event |
| `l1 block <n>` | Inject new L1 block notification |
| `l1 message <json>` | Inject an L1 message |
| `l1 commit <json>` | Inject batch commit |
| `l1 finalize <idx>` | Inject batch finalization |
| `l1 reorg <block>` | Inject L1 reorg |

### Block & Transaction

| Command | Description |
|---------|-------------|
| `build` | Build a new block (sequencer mode) |
| `tx send <to> <value> [idx]` | Send ETH transfer (value in wei, idx = wallet index) |
| `tx pending` | List pending transactions |
| `tx inject <hex>` | Inject raw transaction |

**Example:**

```
scroll-debug [seq:0]> tx send 0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb2 1000000000000000000
Transaction sent!
  Hash: 0xabcd...
  From: 0x1234...
  To:   0x742d...
  Value: 1000000000000000000 wei
Note: Run 'build' to include in a block (sequencer mode)

scroll-debug [seq:0]> build
Block build triggered!
```

**Viewing pending transactions:**

```
scroll-debug [seq:0]> tx pending
Pending Transactions (2):
  [0] hash=0x1234abcd5678... from=0x742d35Cc... nonce=5 gas_price=1000000000
  [1] hash=0xabcdef123456... from=0x742d35Cc... nonce=6 gas_price=1000000000
```

### Wallet

| Command | Description |
|---------|-------------|
| `wallet` | Show wallet address, balance, and nonce |
| `wallet gen` | Generate and list all available wallets |

The toolkit includes pre-funded test wallets. Use `wallet gen` to see all available wallets, then reference them by index in `tx send`:

```
scroll-debug [seq:0]> wallet gen
Generated Wallets (10):
  Chain ID: 222222

  [0] 0x1234567890abcdef...
      Balance: 1000000000000000000000 wei (1000.000000 ETH)

  [1] 0xabcdef1234567890...
      Balance: 1000000000000000000000 wei (1000.000000 ETH)
  ...

scroll-debug [seq:0]> tx send 0x742d... 1000000 2
# Sends from wallet index 2
```

### Network

| Command | Description |
|---------|-------------|
| `peers` | List connected peers and show local enode |
| `peers connect <url>` | Connect to a peer (enode://...) |

### Events

The REPL streams chain events in real-time as they occur:

| Command | Description |
|---------|-------------|
| `events on` | Enable background event stream |
| `events off` | Disable background event stream |
| `events [count]` | Stream next N events (default: 10) |
| `events filter <pat>` | Filter events by type (e.g., `Block*`, `L1*`) |
| `events history [n]` | Show last N events (default: 20) |

**Example with events enabled:**

```
scroll-debug [seq:0]> build
  [EVENT] BlockSequenced { block: 1, hash: 0xabcd... }
  [EVENT] ChainExtended { block: 1 }
Block build triggered!
```

### Custom Actions

Run pre-built or custom actions with full access to the test fixture:

| Command | Description |
|---------|-------------|
| `run list` | List available custom actions |
| `run <name> [args]` | Execute a custom action |

**Built-in Actions:**

| Action | Description |
|--------|-------------|
| `build-blocks <count> [delay_ms]` | Build multiple blocks in sequence |
| `stress-test <tx_count> [build_every]` | Send multiple transactions and build blocks |
| `sync-all` | Send L1 sync event to all nodes |

**Example:**

```
scroll-debug [seq:0]> run build-blocks 10 100
Running action: build-blocks

Building 10 blocks with 100ms delay...
  Block 1 triggered
  Block 2 triggered
  ...
Done! Head is now at block #10
```

### Node Management

When running with multiple nodes (e.g., `--followers 2`):

| Command | Description |
|---------|-------------|
| `node <index>` | Switch active node context |
| `nodes` | List all nodes in fixture |

```
scroll-debug [seq:0]> nodes
Nodes:
  [0] Sequencer *
  [1] Follower
  [2] Follower

scroll-debug [seq:0]> node 1
Switched to node 1 (Follower)

scroll-debug [fol:1]> status
...
```

### Database

| Command | Description |
|---------|-------------|
| `db` | Show database path and access command |

The `db` command shows the SQLite database path and provides a command to access it from another terminal:

```
scroll-debug [seq:0]> db
Database Info:
  Path: /path/to/datadir/db/scroll.db

Access from another terminal:
  sqlite3 /path/to/datadir/db/scroll.db

Useful queries:
  .tables                       -- List all tables
  .schema <table>               -- Show table schema
  SELECT * FROM metadata;       -- View metadata
  SELECT * FROM l2_block ORDER BY number DESC LIMIT 10;
```

The database path is also shown in the `status` command output.

### Other

| Command | Description |
|---------|-------------|
| `help` | Show available commands |
| `exit` | Exit the REPL |

## Creating Custom Actions

You can create custom actions by implementing the `Action` trait. Actions have full access to the `TestFixture`, allowing you to:

- Access all nodes and their RPC interfaces
- Send transactions from test wallets
- Trigger block building
- Inject L1 events
- Query chain state

### Implementing an Action

```rust
use rollup_node::debug_toolkit::actions::{Action, ActionRegistry};
use rollup_node::test_utils::TestFixture;
use async_trait::async_trait;

struct MyCustomAction;

#[async_trait]
impl Action for MyCustomAction {
    fn name(&self) -> &'static str {
        "my-action"
    }

    fn description(&self) -> &'static str {
        "Does something cool with the fixture"
    }

    fn usage(&self) -> Option<&'static str> {
        Some("run my-action [arg1] [arg2]")
    }

    async fn execute(
        &self,
        fixture: &mut TestFixture,
        args: &[String],
    ) -> eyre::Result<()> {
        // Access nodes
        println!("Fixture has {} nodes", fixture.nodes.len());

        // Access wallet
        let wallet = fixture.wallet.lock().await;
        println!("Wallet address: {:?}", wallet.inner.address());
        drop(wallet);

        // Access specific node
        let node = &fixture.nodes[0];
        let status = node.rollup_manager_handle.status().await?;
        println!("Head block: {}", status.l2.fcs.head_block_info().number);

        // Trigger block building (sequencer only)
        if node.is_sequencer() {
            node.rollup_manager_handle.build_block();
        }

        // Inject L1 events
        fixture.l1().sync().await?;

        Ok(())
    }
}
```

### Registering Actions

Add your action to the registry in `crates/node/src/debug_toolkit/actions.rs`:

```rust
impl ActionRegistry {
    pub fn new() -> Self {
        let mut registry = Self { actions: Vec::new() };

        // Built-in actions
        registry.register(Box::new(BuildBlocksAction));
        registry.register(Box::new(StressTestAction));
        registry.register(Box::new(SyncAllAction));

        // Add your custom action here:
        registry.register(Box::new(MyCustomAction));

        registry
    }
}
```

Or register programmatically before running the REPL:

```rust
let fixture = TestFixture::builder()
    .with_chain("dev")
    .sequencer()
    .build()
    .await?;

let mut repl = DebugRepl::new(fixture);
repl.action_registry_mut().register(Box::new(MyCustomAction));
repl.run().await?;
```

## Use Cases

### Hackathon Development

The debug toolkit is ideal for hackathons where you need to:

- Quickly spin up a local Scroll environment
- Test smart contract interactions
- Debug transaction flows
- Simulate L1-L2 message passing

### Integration Testing

Use custom actions to create reproducible test scenarios:

```rust
struct IntegrationTestAction;

#[async_trait]
impl Action for IntegrationTestAction {
    fn name(&self) -> &'static str { "integration-test" }
    fn description(&self) -> &'static str { "Run integration test suite" }

    async fn execute(&self, fixture: &mut TestFixture, _args: &[String]) -> eyre::Result<()> {
        // 1. Sync L1
        fixture.l1().sync().await?;

        // 2. Send some transactions
        // ...

        // 3. Build blocks
        let sequencer = fixture.nodes.iter().find(|n| n.is_sequencer()).unwrap();
        sequencer.rollup_manager_handle.build_block();

        // 4. Verify state
        // ...

        Ok(())
    }
}
```

### Debugging

Inspect chain state interactively:

```
scroll-debug [seq:0]> block 42
Block #42
  Hash:       0xabcd...
  Parent:     0x1234...
  Timestamp:  1705123456
  Gas Used:   21000
  Gas Limit:  20000000
  Txs:        3
    [0] hash=0x1111...
    [1] hash=0x2222...
    [2] hash=0x3333...

scroll-debug [seq:0]> fcs
Forkchoice State:
  Head:
    Number: 42
    Hash:   0xabcd...
  Safe:
    Number: 40
    Hash:   0x5678...
  Finalized:
    Number: 35
    Hash:   0x9abc...
```

## External Tools

### Using Cast

The `status` command shows the HTTP RPC endpoint for each node. You can use [Foundry's `cast`](https://book.getfoundry.sh/cast/) to interact with the node from another terminal:

```
scroll-debug [seq:0]> status
=== Node 0 (Sequencer) ===
Node:
  Database:  /tmp/.tmpXYZ/db/scroll.db
  HTTP RPC:  http://127.0.0.1:62491
...
```

Then in another terminal, use `cast` with the HTTP RPC URL:

```bash
# Get the current block number
cast block-number --rpc-url http://127.0.0.1:62491

# Get block details
cast block latest --rpc-url http://127.0.0.1:62491

# Get an account balance
cast balance 0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb2 --rpc-url http://127.0.0.1:62491

# Send a transaction (using a test private key)
cast send 0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb2 \
  --value 1ether \
  --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  --rpc-url http://127.0.0.1:62491

# Call a contract
cast call 0xContractAddress "balanceOf(address)" 0xUserAddress --rpc-url http://127.0.0.1:62491

# Get transaction receipt
cast receipt 0xTransactionHash --rpc-url http://127.0.0.1:62491
```

This is useful for:
- Testing smart contract deployments and interactions
- Debugging transaction behavior
- Scripting complex test scenarios
- Using familiar Ethereum tooling with your local rollup node
