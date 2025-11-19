# Scroll Rollup Node Docker Compose

This guide explains how to use Docker Compose to launch the Scroll Rollup Node, including standard and shadow-fork
modes.

---

## Prerequisites

- [Docker](https://docs.docker.com/get-docker/) and [Docker Compose](https://docs.docker.com/compose/install/) installed
- Clone this repository
- **L1 RPC endpoint** with API key (from Alchemy, Infura, QuickNode, etc.)
- **Beacon node endpoint** for blob data access

---

## Quick Start: Launching the Node

1. **Navigate to the `docker-compose` directory:**
   ```sh
   cd docker-compose
   ```

2. **Configure your RPC endpoints in the `.env` file:**
   ```sh
   # Edit .env and set your RPC URLs:
   ENV=sepolia  # or mainnet
   L1_URL=https://eth-sepolia.g.alchemy.com/v2/YOUR_API_KEY
   BEACON_URL=https://eth-sepolia.g.alchemy.com/v2/YOUR_API_KEY
   ```
   Replace `YOUR_API_KEY` with your actual API key from your RPC provider.

3. **Start the node and monitoring stack:**
   ```sh
   docker compose up -d
   ```
   This will launch the rollup node, Prometheus, and Grafana.

4. **Access the services:**
    - Rollup Node JSON-RPC: [http://localhost:8545](http://localhost:8545)
    - Rollup Node WebSocket: [ws://localhost:8546](ws://localhost:8546)
    - Prometheus: [http://localhost:19090](http://localhost:19090)
    - Grafana: [http://localhost:13000](http://localhost:13000)

---

## Shadow-Fork Mode

Shadow-fork mode allows you to run the node against a forked L1 chain for testing and development.

### 1. Edit the `.env` file

The file `docker-compose/.env` contains environment variables for configuration:

```
SHADOW_FORK=true
FORK_BLOCK_NUMBER=8700000   # Change to your desired fork block
ENV=sepolia                # Or 'mainnet' for mainnet fork

# Note: L1_URL and BEACON_URL are not required in shadow-fork mode
# The local L1 devnet will be used automatically
```

### 2. Launch with the shadow-fork profile

**Recommended (Docker Compose v1.28+):**

```sh
docker compose --env-file .env --profile shadow-fork up -d
```

- This will start both the L1 devnet and the rollup node in shadow-fork mode.
- The `FORK_BLOCK_NUMBER` and `ENV` variables control the fork point and network.

---

## Stopping the Stack

To stop all services:

```sh
docker compose down
```

To reset your current local network, simply remove all persistent volumes:
```sh
rm -rf volumes
```

---

## Troubleshooting

- Make sure the ports (8545, 8546, 19090, 13000) are not used by other processes.
- If you change environment variables, restart the stack with `docker compose down && docker compose up -d`.
- For shadow-fork mode, always ensure you specify the correct `--env-file` or have the right `.env` file in place.

---

## Configuration Options

### Environment Variables

Key variables in the `.env` file:

- `ENV`: Network to connect to (`dev`, `sepolia`, or `mainnet`)
- `SHADOW_FORK`: Enable shadow-fork mode (`true` or `false`)
- `FORK_BLOCK_NUMBER`: Block number to fork from (shadow-fork mode only)
- `L1_URL`: **Required** - Your L1 RPC endpoint URL (e.g., Alchemy, Infura, QuickNode)
- `BEACON_URL`: **Required** - Your beacon node URL for blob data

### Configuring RPC Endpoints

You **must** provide your own RPC endpoints to run the node:

```env
ENV=sepolia
L1_URL=https://eth-sepolia.g.alchemy.com/v2/YOUR_API_KEY
BEACON_URL=https://eth-sepolia.g.alchemy.com/v2/YOUR_API_KEY
```

**Recommended RPC Providers:**
- [Alchemy](https://www.alchemy.com/) - Free tier available
- [Infura](https://infura.io/) - Free tier available
- [QuickNode](https://www.quicknode.com/) - High-performance nodes

The configuration priority is:
1. User-provided URLs (via `L1_URL`/`BEACON_URL`)
2. Shadow-fork URLs (if `SHADOW_FORK=true`) - uses local L1 devnet
3. Error - Node will not start without valid configuration

## More

- See `docker-compose/docker-compose.yml` for all available services and configuration options.
- For detailed documentation, refer to the project book in `book/src/docker-operations.md`.
- For advanced usage, refer to the official [Docker Compose documentation](https://docs.docker.com/compose/).

