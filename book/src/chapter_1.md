# Introduction

Welcome to the Scroll Rollup Node documentation.

## What is Scroll?

Scroll is a zkRollup on Ethereum that enables scaling while maintaining security and decentralization through
zero-knowledge proofs. By moving computation and state storage off-chain while posting transaction data and validity
proofs to Ethereum L1, Scroll achieves higher throughput and lower transaction costs while inheriting Ethereum's
security guarantees.

## What is the Rollup Node?

The rollup node is responsible for following the Scroll L2 chain using P2P data from the Scroll network, and
consolidating this information with data posted to Ethereum L1.
At its core, the rollup node implements a derivation function: given the L1 chain state, it deterministically produces
the corresponding L2 chain. This allows it to follow the correct L2 chain in case malicious blocks are propagated on
the P2P network.

### Derivation Function

The derivation process works by:

1. **Monitoring L1**: Watching for batch commitments, finalization events, and cross-chain messages posted to Ethereum
2. **Decoding Batches**: Extracting and decoding batch data (including blob data) to reconstruct transaction lists
3. **Building Payloads**: Constructing execution payloads with the appropriate transactions and L1 messages
4. **Executing Blocks**: Applying payloads through the execution engine to advance the L2 state

### Architecture

Built on the Reth framework, the rollup node employs an event-driven architecture where specialized components
communicate through async channels:

- **L1 Watcher**: Tracks L1 events and maintains awareness of chain reorganizations
- **Derivation Pipeline**: Transforms batch data from L1 into executable L2 payloads
- **Engine Driver**: Interfaces with the execution engine via the Engine API
- **Chain Orchestrator**: Coordinates the overall flow from L1 events to L2 blocks
- **Network Layer**: Participates in the Scroll and Ethereum P2P network

### Node Modes

The rollup node can operate in different configurations:

- **Follower Node**: Follows the L2 chain via P2P propagated blocks, consolidated by processing batch data posted to L1
- **Sequencer Node**: Actively sequences new transactions into blocks and posts batches to L1

## About This Documentation

This documentation provides comprehensive guides for operating and understanding the Scroll rollup node, including setup
instructions, configuration options, architecture details, and troubleshooting guidance.
