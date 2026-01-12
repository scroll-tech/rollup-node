# Running a Scroll Sequencer Node

This guide covers how to run a Scroll rollup node in sequencer mode. A sequencer node is responsible for ordering transactions, building new blocks, and proposing them to the network.

## What is a Sequencer Node?

A sequencer node actively produces new blocks for the Scroll L2 chain. Unlike follower nodes that listen for blocks gossiped over L2 P2P and derive blocks from L1 data, sequencer nodes:

- Accept transactions from the mempool
- Order transactions and build new blocks
- Include L1 messages in blocks according to configured inclusion strategy
- Sign blocks with a configured signer (private key or AWS KMS)
- Broadcast blocks to the network

**Note:** Running a sequencer requires authorization. The sequencer's address must be registered as the authorized signer in the L1 system contract for blocks to be accepted by the network.

## Prerequisites

### Hardware Requirements

Sequencer nodes have similar requirements to follower nodes:

- **CPU**: 2+ cores @ 3 GHz
- **Memory**: 16 GB RAM

### Building the Binary

Build the rollup node binary in release mode for production use:

```bash
cargo build --release --bin rollup-node
```

The release binary will be located at `target/release/rollup-node`.

## Signer Configuration

A sequencer node requires a signer to sign blocks. There are two signing methods available:

### Option 1: Private Key File

Store your private key in a file and reference it with the `--signer.key-file` flag.

**Private Key File Format:**
- Hex-encoded private key (64 characters)
- Optional `0x` prefix
- No additional formatting or whitespace

**Example private key file (`/secure/path/sequencer.key`):**
```
0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef
```

**Security Best Practices:**
- Store the key file in a secure location with restricted permissions (`chmod 600`)
- Never commit private keys to version control
- Use file system encryption for the key file
- Regularly rotate sequencer keys
- Consider using hardware security modules (HSM) for production

### Option 2: AWS KMS (not thoroughly tested!!)

Use AWS Key Management Service (KMS) to manage your sequencer's signing key. This is the recommended approach for production deployments.

**Requirements:**
- AWS account with KMS access
- KMS key created with signing capabilities
- IAM permissions for the sequencer node:
  - `kms:GetPublicKey` - Retrieve the public key
  - `kms:Sign` - Sign block hashes

**KMS Key Format:**
```
arn:aws:kms:REGION:ACCOUNT_ID:key/KEY_ID
```

**Example:**
```
arn:aws:kms:us-west-2:123456789012:key/12345678-1234-1234-1234-123456789012
```

**Benefits of AWS KMS:**
- Keys never leave AWS infrastructure
- Centralized key management and rotation
- Audit logging of signing operations
- Fine-grained access control via IAM

### Test Mode (Development Only)

For development and testing, you can bypass signer requirements with the `--test` flag:

```bash
--test
```

**Warning:** Never use test mode in production. It disables signature verification and is only suitable for local development.

## Sequencer Configuration Flags

### Core Sequencer Flags

- `--sequencer.enabled`: Enable sequencer mode (default: `false`)
- `--sequencer.auto-start`: Automatically start sequencing on node startup (default: `false`)

### Block Production Configuration

- `--sequencer.block-time <MS>`: Time between blocks in milliseconds (default: `1000`)
- `--sequencer.payload-building-duration <MS>`: Time allocated for building each payload in milliseconds (default: `800`)
- `--sequencer.allow-empty-blocks`: Allow production of empty blocks when no transactions are available (default: `false`)

### Fee Configuration

- `--sequencer.fee-recipient <ADDRESS>`: Address to receive block rewards and transaction fees (default: `0x5300000000000000000000000000000000000005` - Scroll fee vault)

### L1 Message Inclusion

The sequencer can include L1 messages in blocks using different strategies:

- `--sequencer.l1-inclusion-mode <MODE>`: Strategy for including L1 messages (default: `"finalized:2"`)

**Available modes:**
- `finalized:N` - Include messages from finalized L1 blocks which have a block number <= current finalized - N (e.g., `finalized:2`)
- `depth:N` - Include messages from L1 blocks with a block number <= current head - N (e.g., `depth:10`)

**Example:**
```bash
--sequencer.l1-inclusion-mode finalized:2
```

- `--sequencer.max-l1-messages <N>`: Override maximum L1 messages per block (optional)

### Signer Configuration

**Mutually exclusive options (choose one):**

- `--signer.key-file <PATH>`: Path to hex-encoded private key file
- `--signer.aws-kms-key-id <ARN>`: AWS KMS key ID or ARN

## Example Configurations

### Development Sequencer (Test Mode)

For local development and testing:

```bash
./target/release/rollup-node node \
  --chain dev \
  --datadir ./data/sequencer \
  --test \
  --sequencer.enabled \
  --sequencer.auto-start \
  --sequencer.block-time 1000 \
  --http \
  --http.addr 0.0.0.0 \
  --http.port 8545 \
  --http.api admin,debug,eth,net,trace,txpool,web3,rpc,reth,ots,flashbots,miner,mev
```

**Notes:**
- Uses `--test` flag to bypass signer requirement
- Suitable for local development only
- Genesis funds available for testing

### Production Sequencer with Private Key

For production deployment with private key file:

```bash
./target/release/rollup-node node \
  --chain scroll-mainnet \
  --datadir /var/lib/scroll-sequencer \
  --l1.url https://eth-mainnet.g.alchemy.com/v2/YOUR_API_KEY \
  --blob.s3_url https://scroll-mainnet-blob-data.s3.us-west-2.amazonaws.com/ \
  --sequencer.enabled \
  --sequencer.auto-start \
  --sequencer.block-time 1000 \
  --sequencer.payload-building-duration 800 \
  --sequencer.l1-inclusion-mode finalized:2 \
  --sequencer.fee-recipient 0x5300000000000000000000000000000000000005 \
  --signer.key-file /secure/path/sequencer.key \
  --http \
  --http.addr 0.0.0.0 \
  --http.port 8545 \
  --http.api eth,net,web3,debug,trace
```

### Production Sequencer with AWS KMS

For production deployment with AWS KMS:

```bash
./target/release/rollup-node node \
  --chain scroll-mainnet \
  --datadir /var/lib/scroll-sequencer \
  --l1.url https://eth-mainnet.g.alchemy.com/v2/YOUR_API_KEY \
  --blob.s3_url https://scroll-mainnet-blob-data.s3.us-west-2.amazonaws.com/ \
  --sequencer.enabled \
  --sequencer.auto-start \
  --sequencer.block-time 1000 \
  --sequencer.payload-building-duration 800 \
  --sequencer.l1-inclusion-mode finalized:2 \
  --sequencer.fee-recipient 0x5300000000000000000000000000000000000005 \
  --signer.aws-kms-key-id arn:aws:kms:us-west-2:123456789012:key/YOUR-KEY-ID \
  --http \
  --http.addr 0.0.0.0 \
  --http.port 8545 \
  --http.api eth,net,web3,debug,trace
```

### Sepolia Testnet Sequencer

For testing on Sepolia testnet:

```bash
./target/release/rollup-node node \
  --chain scroll-sepolia \
  --datadir /var/lib/scroll-sequencer-sepolia \
  --l1.url https://eth-sepolia.g.alchemy.com/v2/YOUR_API_KEY \
  --blob.s3_url https://scroll-sepolia-blob-data.s3.us-west-2.amazonaws.com/ \
  --sequencer.enabled \
  --sequencer.auto-start \
  --signer.key-file /path/to/sepolia-sequencer.key \
  --http \
  --http.addr 0.0.0.0 \
  --http.port 8545 \
  --http.api eth,net,web3
```

## Verifying Sequencer Operation

### Check Sequencer is Producing Blocks

Monitor the logs for block production:

```bash
# Look for sequencer-related log entries
RUST_LOG=info,scroll=debug,rollup=debug ./target/release/rollup-node node ...
```

Expected log patterns:
- `Built payload` - Payload construction
- `Signed block` - Block signing
- `Broadcast block` - Block propagation

### Query Latest Block

Verify the sequencer is producing new blocks:

```bash
# Check block number is incrementing
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_blockNumber",
    "params": [],
    "id": 1
  }'
```

Run this command multiple times (with `--sequencer.block-time` interval) to confirm blocks are being produced.

## Logging and Debugging

### Recommended Log Configuration for Sequencer

For production sequencer operation with detailed logging:

```bash
RUST_LOG=info,scroll=debug,rollup_node::sequencer=trace,scroll::engine=debug
```

### Sequencer-Specific Log Targets

**Sequencer Operations:**
```bash
RUST_LOG=rollup_node::sequencer=trace
```

Monitor payload building, transaction selection, and block production.

**Block Signing:**
```bash
RUST_LOG=rollup_node::signer=debug
```

Track signing operations and signer interactions.

**L1 Message Processing:**
```bash
RUST_LOG=scroll::watcher=debug
```

Monitor L1 message detection and inclusion.

## Troubleshooting

### Sequencer Not Producing Blocks

**Symptoms:** No new blocks appearing, logs show no sequencer activity

**Possible causes:**
1. `--sequencer.auto-start` not enabled - Start sequencing manually via RPC
2. Signer configuration error - Check `--signer.*` flags
3. Not authorized on L1 - Verify sequencer address is registered in system contract
4. Insufficient gas for transactions - Check mempool has valid transactions
5. `--sequencer.allow-empty-blocks` not set and no transactions available

**Solutions:**
- Enable auto-start: `--sequencer.auto-start`
- Verify signer configuration in logs
- Check authorization on L1 system contract
- Review logs with `RUST_LOG=rollup_node::sequencer=trace`

### Signer Errors

**Symptoms:** Errors mentioning "signer", "signature", or "key"

**With `--signer.key-file`:**
- Verify file exists and is readable
- Check file permissions (`chmod 600`)
- Validate hex format (64 characters, optional `0x` prefix)
- Ensure no extra whitespace or newlines

**With `--signer.aws-kms-key-id`:**
- Verify AWS credentials are configured
- Check IAM permissions (`kms:GetPublicKey`, `kms:Sign`)
- Confirm KMS key ARN is correct
- Check network connectivity to AWS
- Review AWS CloudTrail logs for KMS denials

### Block Signing Failed

**Symptoms:** Logs show "failed to sign"

**Solutions:**
1. Check signer configuration is correct
2. For AWS KMS, verify network connectivity and IAM permissions
3. Ensure the signing key has not been disabled or deleted
4. Check system time is synchronized (important for KMS)

### L1 Message Inclusion Errors

**Symptoms:** Errors related to L1 messages, blocks rejected

**Solutions:**
- Verify `--l1.url` is accessible and synced
- Check `--sequencer.l1-inclusion-mode` configuration appropriately
- Ensure L1 messages are being detected (check L1 watcher logs): `RUST_LOG=scroll::watcher=debug`

## Additional Resources

- [Running a Follower Node](./running-a-node.md) - Configuration for non-sequencer nodes
- [Docker Operations](./docker-operations.md) - Running node in Docker
- [Sequencer Migration Guide](../sequencer-migration/README.md) - Migrating from l2geth to rollup-node
