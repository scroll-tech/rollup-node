# Debug Toolkit

The Debug Toolkit is an interactive REPL (Read-Eval-Print Loop) for debugging, development, and hackathon scenarios. It allows you to spin up local follower nodes that connect to a remote sequencer and L1, run tests, execute scripts, and inspect chain state.

## Getting Started

### Source Code

The debug toolkit is available on the `feat/debug-toolkit` branch:

**Repository:** [https://github.com/scroll-tech/rollup-node/tree/feat/debug-toolkit](https://github.com/scroll-tech/rollup-node/tree/feat/debug-toolkit)

```bash
git clone https://github.com/scroll-tech/rollup-node.git
cd rollup-node
git checkout feat/debug-toolkit
```

### Building

Build with the `debug-toolkit` feature flag:

```bash
cargo build -p rollup-node --features debug-toolkit --release
```

## Connecting to a Remote Network

The primary use case is connecting local follower nodes to a remote sequencer and L1. This allows you to run tests and scripts against a live network.

### Network Connection Info

```
L1 RPC:           http://ec2-54-167-214-30.compute-1.amazonaws.com:8545
L1 Private Key:   0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
Sequencer HTTP:   http://ec2-54-175-126-206.compute-1.amazonaws.com:8545
Sequencer Enode:  enode://3322bb29bba1f30f3bb40e816779f1be8ab3c14a5d14ff6c76d0585a63bdcc4ba25008be138780dafd03cb3e3ae4546da9a566b4ff9f1d237fa5d3d79bfdd219@54.175.126.206:30303
Signer:           0xb674ff99cca262c99d3eab5b32796a99188543da
Genesis:          tests/l2reth-genesis-e2e.json
```

### Connect to Remote Network

```bash
cargo run --features debug-toolkit --bin scroll-debug -- \
    --followers 2 \
    --chain tests/l2reth-genesis-e2e.json \
    --bootnodes enode://3322bb29bba1f30f3bb40e816779f1be8ab3c14a5d14ff6c76d0585a63bdcc4ba25008be138780dafd03cb3e3ae4546da9a566b4ff9f1d237fa5d3d79bfdd219@54.175.126.206:30303 \
    --l1-url http://ec2-54-167-214-30.compute-1.amazonaws.com:8545 \
    --valid-signer 0xb674ff99cca262c99d3eab5b32796a99188543da
```

This creates local follower nodes that:
- Connect to the remote sequencer via P2P (`--bootnodes`)
- Sync L1 state from the remote L1 RPC (`--l1-url`)
- Validate blocks using the network's authorized signer (`--valid-signer`)
- Use the matching genesis configuration (`--chain`)

### CLI Options Explained

| Option | Description |
|--------|-------------|
| `--chain <name\|path>` | Genesis configuration: `dev`, `scroll-sepolia`, `scroll-mainnet`, or path to JSON file |
| `--sequencer` | Enable local sequencer mode |
| `--followers <n>` | Number of local follower nodes to spin up (can be any number) |
| `--bootnodes <enode>` | Remote sequencer enode URL to connect to |
| `--l1-url <url>` | Remote L1 RPC endpoint |
| `--valid-signer <addr>` | Authorized block signer address for consensus validation |
| `--log-file <path>` | Path to log file (default: `./scroll-debug-<pid>.log`) |

## Local-Only Mode

You can also run a fully local environment with a mock L1 and local sequencer for offline development:

```bash
cargo run --features debug-toolkit --bin scroll-debug -- \
    --chain dev \
    --sequencer \
    --followers 2
```

This creates:
- **Node 0**: Local sequencer (produces blocks)
- **Node 1-N**: Local followers (receive blocks via P2P)

With mock L1, you must manually sync before building blocks:

```
scroll-debug [seq:0]> l1 sync
L1 synced event sent

scroll-debug [seq:0]> build
Block build triggered!
```

## Using the Network Handle

The `TestFixture` provides access to network handles for programmatic control. This is useful for writing custom actions and tests:

```rust
use rollup_node::test_utils::TestFixture;

async fn example(fixture: &TestFixture) -> eyre::Result<()> {
    // Access a node's network handle
    let node = &fixture.nodes[0];
    let network_handle = node.rollup_manager_handle.get_network_handle().await?;

    // Get local node info
    let local_record = network_handle.local_node_record();
    println!("Local enode: {}", local_record);

    // Access the inner network handle for P2P operations
    let inner = network_handle.inner();

    // Get connected peers
    let peers = inner.get_all_peers().await?;
    println!("Connected to {} peers", peers.len());

    // Add a peer
    inner.add_peer(peer_id, socket_addr);

    Ok(())
}
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
scroll-debug [fol:0]> status
=== Node 0 (Follower) ===
Node:
  Database:  /tmp/.tmpXYZ/db/scroll.db
  HTTP RPC:  http://127.0.0.1:62491
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

These commands allow you to simulate L1 events (useful in local mode with mock L1):

| Command | Description |
|---------|-------------|
| `l1 status` | Show L1 sync state |
| `l1 sync` | Inject L1 synced event |
| `l1 block <n>` | Inject new L1 block notification |
| `l1 reorg <block>` | Inject L1 reorg |

### Block & Transaction

| Command | Description |
|---------|-------------|
| `build` | Build a new block (local sequencer mode only) |
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
```

### Wallet

| Command | Description |
|---------|-------------|
| `wallet` | Show wallet address, balance, and nonce |
| `wallet gen` | Generate and list all available wallets |

The toolkit includes pre-funded test wallets:

```
scroll-debug [fol:0]> wallet gen
Generated Wallets (10):
  Chain ID: 222222

  [0] 0x1234567890abcdef...
      Balance: 1000000000000000000000 wei (1000.000000 ETH)

  [1] 0xabcdef1234567890...
      Balance: 1000000000000000000000 wei (1000.000000 ETH)
  ...
```

### Network

| Command | Description |
|---------|-------------|
| `peers` | List connected peers and show local enode |
| `peers connect <url>` | Connect to a peer (enode://...) |

**Example:**

```
scroll-debug [fol:0]> peers
Local Node:
  Peer ID: 0x1234...
  Enode:   enode://abcd...@127.0.0.1:30303

Connected Peers (1):
  0x3322bb29...
    Address:    54.175.126.206:30303
    Client:     scroll-reth/v1.0.0
```

### Events

The REPL streams chain events in real-time:

| Command | Description |
|---------|-------------|
| `events on` | Enable background event stream |
| `events off` | Disable background event stream |
| `events filter <pat>` | Filter events by type (e.g., `Block*`, `L1*`) |
| `events history [n]` | Show last N events (default: 20) |

### Custom Actions

Run pre-built or custom actions:

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

### Node Management

Switch between nodes when running multiple followers:

| Command | Description |
|---------|-------------|
| `node <index>` | Switch active node context |
| `nodes` | List all nodes in fixture |

```
scroll-debug [fol:0]> nodes
Nodes:
  [0] Follower *
  [1] Follower
  [2] Follower

scroll-debug [fol:0]> node 1
Switched to node 1 (Follower)
```

### Database

| Command | Description |
|---------|-------------|
| `db` | Show database path and access command |

```
scroll-debug [fol:0]> db
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

### Logs

| Command | Description |
|---------|-------------|
| `logs` | Show log file path and tail command |

Tracing logs are written to a file to keep the REPL display clean:

```
scroll-debug [fol:0]> logs
Log File:
  Path: ./scroll-debug-12345.log

View logs in another terminal:
  tail -f ./scroll-debug-12345.log
```

### Other

| Command | Description |
|---------|-------------|
| `help` | Show available commands |
| `exit` | Exit the REPL |

## Creating Custom Actions

You can create custom actions by implementing the `Action` trait. Actions have full access to the `TestFixture`:

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

        // Access network handle
        let node = &fixture.nodes[0];
        let network_handle = node.rollup_manager_handle.get_network_handle().await?;
        println!("Connected peers: {}", network_handle.inner().num_connected_peers());

        // Access wallet
        let wallet = fixture.wallet.lock().await;
        println!("Wallet address: {:?}", wallet.inner.address());
        drop(wallet);

        // Query chain state
        let status = node.rollup_manager_handle.status().await?;
        println!("Head block: {}", status.l2.fcs.head_block_info().number);

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

## Useful Cast Commands

Use [Foundry's `cast`](https://book.getfoundry.sh/cast/) to interact with the network. The `status` command in the REPL shows the HTTP RPC endpoint for your local nodes.

### Check Block Status

```bash
# Check latest L1 block
cast block latest --rpc-url http://ec2-54-167-214-30.compute-1.amazonaws.com:8545

# Check latest L2 block (sequencer)
cast block latest --rpc-url http://ec2-54-175-126-206.compute-1.amazonaws.com:8545
```

### Check Sync Status

```bash
# L2 sequencer sync status
cast rpc rollupNode_status --rpc-url http://ec2-54-175-126-206.compute-1.amazonaws.com:8545 | jq
```

### Check and Manage Peers

```bash
# Check connected peers
cast rpc admin_peers --rpc-url http://ec2-54-175-126-206.compute-1.amazonaws.com:8545

# Get node info (enode URL)
cast rpc admin_nodeInfo --rpc-url http://ec2-54-175-126-206.compute-1.amazonaws.com:8545 | jq -r '.enode'
```

### Enable Sequencing

```bash
cast rpc rollupNodeAdmin_enableAutomaticSequencing --rpc-url http://ec2-54-175-126-206.compute-1.amazonaws.com:8545
```

### Send Transactions

```bash
# Get wallet address from private key
cast wallet address --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80

# Check balance on L2
cast balance 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 --ether --rpc-url http://ec2-54-175-126-206.compute-1.amazonaws.com:8545

# Send L2 transaction
cast send 0x0000000000000000000000000000000000000002 \
  --rpc-url http://ec2-54-175-126-206.compute-1.amazonaws.com:8545 \
  --value 0.00001ether \
  --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
```

### L1 to L2 Bridge (Send L1 Message)

```bash
# Send message from L1 to L2 via the messenger contract
cast send --rpc-url http://ec2-54-167-214-30.compute-1.amazonaws.com:8545 \
  --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  --legacy --gas-price 0.1gwei --gas-limit 200000 --value 0.001ether \
  "0x8A791620dd6260079BF849Dc5567aDC3F2FdC318" \
  "sendMessage(address _to, uint256 _value, bytes memory _message, uint256 _gasLimit)" \
  0x0000000000000000000000000000000000000002 0x1 0x 200000

# Check L1 message queue index
cast call --rpc-url http://ec2-54-167-214-30.compute-1.amazonaws.com:8545 \
  "0xDc64a140Aa3E981100a9becA4E685f962f0cF6C9" \
  "nextCrossDomainMessageIndex()(uint256)"
```

### Contract Addresses

| Contract | Address |
|----------|---------|
| L1 Messenger | `0x8A791620dd6260079BF849Dc5567aDC3F2FdC318` |
| L1 Message Queue | `0xDc64a140Aa3E981100a9becA4E685f962f0cF6C9` |
