# Running with Docker Compose

This guide covers how to run the Scroll rollup node using Docker Compose. The Docker setup provides a complete
environment including the rollup node, monitoring infrastructure, and optional L1 devnet for shadow-fork testing.

## Overview

The Docker Compose stack includes the following services:

- **rollup-node**: The main Scroll rollup node
- **prometheus**: Metrics collection and time-series database
- **grafana**: Visualization dashboard for metrics
- **l1-devnet** (optional): Local L1 Ethereum node for shadow-fork mode

## Prerequisites

- [Docker](https://docs.docker.com/get-docker/) installed (version 20.10 or later)
- [Docker Compose](https://docs.docker.com/compose/install/) installed (version 1.28 or later)
- At least 16 GB RAM
- Sufficient disk space for chain data

## Quick Start

### 1. Navigate to Docker Compose Directory

```bash
cd docker-compose
```

### 2. Configure Environment

The docker-compose setup uses a `.env` file for configuration. You **must** configure your own L1 RPC and beacon node
endpoints.

Key environment variables:

- `ENV`: The network to connect to (`sepolia`, `mainnet`, or `dev`)
- `SHADOW_FORK`: Enable shadow-fork mode (`true` or `false`)
- `FORK_BLOCK_NUMBER`: Block number to fork from (when shadow-fork is enabled)
- `L1_URL`: Your L1 Ethereum RPC endpoint URL (e.g., Alchemy, Infura, QuickNode)
- `BEACON_URL`: Your beacon node URL for blob data

**Note**: You must provide your own RPC endpoints. Configure these in your `.env` file before starting the stack.

### 3. Start the Stack

For standard operation (following public networks):

```bash
docker compose up -d
```

For shadow-fork mode:

```bash
docker compose --profile shadow-fork up -d
```

### 4. Access the Services

Once running, the following endpoints are available:

- **Rollup Node JSON-RPC**: [http://localhost:8545](http://localhost:8545)
- **Rollup Node WebSocket**: [ws://localhost:8546](ws://localhost:8546)
- **Rollup Node Metrics**: [http://localhost:6060/metrics](http://localhost:6060/metrics)
- **Prometheus UI**: [http://localhost:19090](http://localhost:19090)
- **Grafana Dashboards**: [http://localhost:13000](http://localhost:13000)

## Operating Modes

### Standard Mode (Follower Node)

Standard mode connects the rollup node to public Scroll networks (Sepolia or Mainnet).

#### Sepolia Testnet

Edit your `.env` file with your RPC endpoints:

```env
ENV=sepolia
SHADOW_FORK=false
L1_URL=https://eth-sepolia.g.alchemy.com/v2/YOUR_API_KEY
BEACON_URL=https://eth-sepolia.g.alchemy.com/v2/YOUR_API_KEY
```

Replace `YOUR_API_KEY` with your actual API key from your RPC provider (Alchemy, Infura, QuickNode, etc.).

Start the services:

```bash
docker compose up -d
```

The node will:

- Connect to your configured L1 RPC endpoint
- Connect to your configured beacon node
- Sync from the Scroll Sepolia network
- Connect to trusted peers on the network

#### Mainnet

Edit your `.env` file with your RPC endpoints:

```env
ENV=mainnet
SHADOW_FORK=false
L1_URL=https://eth-mainnet.g.alchemy.com/v2/YOUR_API_KEY
BEACON_URL=https://eth-mainnet.g.alchemy.com/v2/YOUR_API_KEY
```

Replace `YOUR_API_KEY` with your actual API key from your RPC provider (Alchemy, Infura, QuickNode, etc.).

Start the services:

```bash
docker compose up -d
```

The node will:

- Connect to your configured L1 RPC endpoint
- Connect to your configured beacon node
- Sync from the Scroll Mainnet network
- Connect to trusted peers on the network

### Shadow-Fork Mode

Shadow-fork mode runs the rollup node against a forked L1 chain, useful for testing and development without affecting
the live network.

#### Configuration

Edit your `.env` file:

```env
ENV=sepolia  # or mainnet
SHADOW_FORK=true
FORK_BLOCK_NUMBER=8700000  # Adjust to your desired fork point
```

#### Starting Shadow-Fork

```bash
docker compose --profile shadow-fork up -d
```

This will:

1. Start an L1 devnet (Anvil) forked from the specified block
2. Start the rollup node configured to use the local L1 devnet
3. Start Prometheus and Grafana for monitoring

#### How Shadow-Fork Works

The `l1-devnet` service uses Foundry's Anvil to create a local fork:

```bash
anvil --fork-url <SCROLL_L1_RPC> \
      --fork-block-number <FORK_BLOCK_NUMBER> \
      --chain-id <CHAIN_ID> \
      --host 0.0.0.0 \
      --block-time 12
```

The rollup node then connects to this local L1 at `http://l1-devnet:8545` instead of the public L1 RPC.

#### Use Cases

Shadow-fork mode is ideal for:

- Testing node behavior at specific block heights
- Debugging derivation issues
- Development without consuming testnet resources
- Simulating historical scenarios

### Development Mode

For local development with a completely isolated environment:

```env
ENV=dev
```

This mode:

- Uses the `dev` chain spec
- Enables sequencer mode
- Bypasses signing requirements with `--test` flag
- Disables peer discovery
- Sets fast block times (250ms)

### Configuring RPC Endpoints

You **must** configure your own L1 RPC and beacon node URLs using a provider of your choice:

**Recommended Providers:**

- [Alchemy](https://www.alchemy.com/) - Offers free tier with generous limits
- [Infura](https://infura.io/) - Reliable infrastructure with free tier
- [QuickNode](https://www.quicknode.com/) - High-performance nodes
- Your own self-hosted L1 archive node

**Requirements:**

- L1 RPC endpoint must support Ethereum mainnet or Sepolia testnet
- Beacon node must provide EIP-4844 blob data access
- Both endpoints should be reliable with good uptime

#### Configuration Priority

The launch script determines which URLs to use in this order:

1. **User-provided URLs** (via `L1_URL` or `BEACON_URL`) - highest priority
2. **Shadow-fork URLs** (if `SHADOW_FORK=true`) - uses local L1 devnet
3. **Error** - If no URLs are configured and not in shadow-fork mode, the node will fail to start

#### Example: Using Alchemy for Sepolia

Edit your `.env` file:

```env
ENV=sepolia
SHADOW_FORK=false
L1_URL=https://eth-sepolia.g.alchemy.com/v2/YOUR_API_KEY
BEACON_URL=https://eth-sepolia.g.alchemy.com/v2/YOUR_API_KEY
```

Then start:

```bash
docker compose up -d
```

#### Verifying RPC Connectivity

After starting the node, verify connectivity to your RPC endpoints by checking the logs:

```bash
docker compose logs rollup-node | grep -i "l1\|beacon"
```

You should see logs indicating successful connection to your configured endpoints. If you see connection errors, verify
your URLs and API keys are correct.

## Service Details

### Rollup Node Service

**Image**: `scrolltech/rollup-node:v1.0.5`

**Port Mappings**:

- `8545`: JSON-RPC interface
- `8546`: WebSocket interface
- `6060`: Metrics endpoint

**Volumes**:

- `./volumes/l2reth`: Node data directory (chain state, database)
- `./launch_rollup_node.bash`: Entrypoint script (read-only)

**Configuration**:

The `launch_rollup_node.bash` script configures the node based on the `ENV` variable:

- **dev**: Local sequencer mode with fast block times
- **sepolia**: Sepolia follower with optional shadow-fork
- **mainnet**: Mainnet follower with optional shadow-fork

Key command-line flags used:

```bash
--chain <CHAIN>           # Chain specification
--datadir=/l2reth         # Data directory
--metrics=0.0.0.0:6060    # Metrics endpoint
--disable-discovery       # P2P discovery disabled in Docker
--http --http.addr=0.0.0.0 --http.port=8545
--ws --ws.addr=0.0.0.0 --ws.port=8546
--http.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev
--l1.url <URL>                      # L1 RPC endpoint
--blob.beacon_node_urls <URLS>      # Beacon node endpoint
--blob.s3_url <URL>                 # S3 Blob URL
--trusted-peers <PEERS>             # Trusted P2P peers
```

### L1 Devnet Service (Shadow-Fork Only)

**Image**: `ghcr.io/foundry-rs/foundry:v1.2.3`

**Port Mappings**:

- `8543`: JSON-RPC interface (mapped from internal 8545)
- `8544`: WebSocket interface (mapped from internal 8546)

**Volumes**:

- `./volumes/l1devnet`: Anvil state directory
- `./launch_l1.bash`: Entrypoint script (read-only)

**Profile**: `shadow-fork` (only starts when this profile is active)

### Prometheus Service

**Image**: `prom/prometheus:v3.3.1`

**Port Mappings**:

- `19090`: Prometheus web UI

**Volumes**:

- `./resource/prometheus.yml`: Configuration file (read-only)
- `./volumes/prometheus`: Time-series database storage

**Configuration**:

Prometheus scrapes metrics from the rollup node every 10 seconds at `http://rollup-node:6060/metrics`. The configuration
includes:

- **Scrape interval**: 10 seconds
- **Retention time**: 1 day
- **Retention size**: 512 MB
- **WAL compression**: Enabled

### Grafana Service

**Image**: `grafana/grafana:12.0.2`

**Port Mappings**:

- `13000`: Grafana web UI (mapped from internal 3000)

**Volumes**:

- `./resource/grafana-datasource.yml`: Prometheus datasource config (read-only)
- `./resource/grafana-dashboard-providers.yml`: Dashboard provider config (read-only)
- `./resource/dashboards/`: Pre-built dashboard JSON files (read-only)
- `./volumes/grafana`: Grafana database and settings

**Configuration**:

- **Anonymous access**: Enabled with Admin role (for easy local access)
- **Default home dashboard**: Overview dashboard
- **Pre-provisioned dashboards**:
    - `overview.json`: High-level node metrics
    - `performance.json`: Performance and throughput metrics
    - `state_history.json`: State and history metrics
    - `transaction_pool.json`: Transaction pool metrics
    - `rollup_node.json`: Rollup-specific metrics

## Monitoring and Observability

### Accessing Grafana Dashboards

1. Open [http://localhost:13000](http://localhost:13000) in your browser
2. No login required (anonymous access enabled)
3. Select from pre-configured dashboards in the sidebar

### Available Dashboards

#### Overview Dashboard

Provides high-level metrics including:

- Block production rate
- Sync status
- L1/L2 block heights
- Network peer count

#### Performance Dashboard

Detailed performance metrics:

- CPU and memory usage
- Database operations per second
- RPC request latency
- Block processing time

#### Transaction Pool Dashboard

Transaction pool monitoring:

- Pending transaction count
- Transaction pool size
- Transaction arrival rate
- Gas price distribution

#### Rollup Node Dashboard

Rollup-specific metrics:

- L1 batch processing
- Derivation pipeline throughput
- Engine API calls
- Consolidation status

### Prometheus Queries

Access Prometheus at [http://localhost:19090](http://localhost:19090) to run custom queries.

Example queries:

```promql
# Current block height
scroll_block_height

# Block processing rate (blocks per second)
rate(scroll_blocks_processed_total[1m])

# L1 batch processing time
histogram_quantile(0.95, scroll_l1_batch_processing_seconds_bucket)

# RPC request rate by method
rate(scroll_rpc_requests_total[5m])
```

### Viewing Logs

#### All Services

```bash
docker compose logs -f
```

#### Specific Service

```bash
docker compose logs -f rollup-node
docker compose logs -f prometheus
docker compose logs -f grafana
```

#### With Timestamps

```bash
docker compose logs -f -t rollup-node
```

#### Last N Lines

```bash
docker compose logs --tail=100 rollup-node
```

## Volume Management

### Data Persistence

The Docker Compose setup uses local volumes in `./volumes/` to persist data:

```
volumes/
├── l1devnet/      # L1 devnet state (shadow-fork only)
├── l2reth/        # Rollup node data (chain state, database)
├── prometheus/    # Prometheus time-series data
└── grafana/       # Grafana configuration and dashboards
```

### Backing Up Data

To backup your node data:

```bash
# Stop the services first
docker compose down

# Create a backup
tar -czf backup-$(date +%Y%m%d).tar.gz volumes/

# Restart services
docker compose up -d
```

### Resetting Node Data

To completely reset and resync:

```bash
# Stop services
docker compose down

# Remove all volumes
rm -rf volumes/

# Restart (will create fresh volumes)
docker compose up -d
```

### Resetting Specific Services

```bash
# Reset only L2 node data
docker compose down
rm -rf volumes/l2reth/
docker compose up -d

# Reset only monitoring data
docker compose down
rm -rf volumes/prometheus/ volumes/grafana/
docker compose up -d
```

## Network Configuration

### Port Usage

The Docker Compose stack uses the following host ports:

| Service     | Port  | Protocol  | Purpose                    |
|-------------|-------|-----------|----------------------------|
| rollup-node | 8545  | HTTP      | JSON-RPC API               |
| rollup-node | 8546  | WebSocket | WebSocket API              |
| rollup-node | 6060  | HTTP      | Metrics endpoint           |
| l1-devnet   | 8543  | HTTP      | L1 JSON-RPC (shadow-fork)  |
| l1-devnet   | 8544  | WebSocket | L1 WebSocket (shadow-fork) |
| prometheus  | 19090 | HTTP      | Prometheus UI              |
| grafana     | 13000 | HTTP      | Grafana UI                 |

## Troubleshooting

### Rollup Node Not Syncing

**Check L1 connectivity:**

For shadow-fork mode, ensure l1-devnet is running:

```bash
docker compose ps l1-devnet
```

For standard mode, verify L1 RPC endpoint is accessible.

**Check logs for derivation errors:**

```bash
docker compose logs -f rollup-node | grep -i error
```

**Verify beacon node connectivity:**

```bash
# From within the container
docker compose exec rollup-node curl http://l1reth-cl.sepolia.scroll.tech:5052/eth/v1/node/health
```

### Shadow-Fork L1 Devnet Issues

**Check Anvil is forking correctly:**

```bash
docker compose logs l1-devnet
```

**Verify fork block number exists:**

Ensure `FORK_BLOCK_NUMBER` is not ahead of the current L1 chain tip.

**Test L1 devnet connectivity:**

```bash
curl -X POST http://localhost:8543 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_blockNumber",
    "params": [],
    "id": 1
  }'
```

## More

For general node operation and configuration, see the [Running a Node](./running-a-node.md) guide.
