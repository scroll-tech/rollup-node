# Scroll Rollup Node Docker Compose

This guide explains how to use Docker Compose to launch the Scroll Rollup Node, including standard and shadow-fork
modes.

---

## Prerequisites

- [Docker](https://docs.docker.com/get-docker/) and [Docker Compose](https://docs.docker.com/compose/install/) installed
- Clone this repository

---

## Quick Start: Launching the Node

1. **Navigate to the `docker-compose` directory:**
   ```sh
   cd docker-compose
   ```

2. **Start the node and monitoring stack:**
   ```sh
   docker compose up -d
   ```
   This will launch the rollup node, Prometheus, and Grafana with default settings.

3. **Access the services:**
    - Rollup Node JSON-RPC: [http://localhost:8545](http://localhost:8545)
    - Rollup Node WebSocket: [ws://localhost:8546](ws://localhost:8546)
    - Prometheus: [http://localhost:19090](http://localhost:19090)
    - Grafana: [http://localhost:13000](http://localhost:13000)

---

## Shadow-Fork Mode

Shadow-fork mode allows you to run the node against a forked L1 chain for testing and development.

### 1. Edit the `.env` file

The file `docker-compose/.env` contains environment variables for enabling shadow-fork mode:

```
SHADOW_FORK=true
FORK_BLOCK_NUMBER=8700000   # Change to your desired fork block
ENV=sepolia                # Or 'mainnet' for mainnet fork
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

---

## Troubleshooting

- Make sure the ports (8545, 8546, 19090, 13000) are not used by other processes.
- If you change environment variables, restart the stack with `docker compose down && docker compose up -d`.
- For shadow-fork mode, always ensure you specify the correct `--env-file` or have the right `.env` file in place.

---

## More

- See `docker-compose/docker-compose.yml` for all available services and configuration options.
- For advanced usage, refer to the official [Docker Compose documentation](https://docs.docker.com/compose/).

