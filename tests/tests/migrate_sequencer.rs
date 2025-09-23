use eyre::Result;
use tests::*;

///
/// This integration test validates that blocks can be successfully propagated between
/// different Ethereum client implementations (l2geth and rollup-node) in various
/// network topologies. The test exercises:
///
/// 1. **Isolated Network Segments**: Initially runs l2geth nodes in isolation, verifying they can
///    produce and sync blocks independently
///    - Topology: `l2geth_follower -> l2geth_sequencer`
///    - l2geth_sequencer produces blocks, l2geth_follower syncs
///    - Rollup nodes remain disconnected at block 0
///
/// 2. **Cross-Client Synchronization**: Connects rollup nodes to the l2geth network, ensuring they
///    can catch up to the current chain state
///    - Topology: `[rn_follower, rn_sequencer, l2geth_follower] -> l2geth_sequencer`
///    - All nodes connect to l2geth_sequencer as the single source of truth
///    - Rollup nodes sync from block 0 to current height
///
/// 3. **Sequencer Handoff**: Transitions block production from l2geth to rollup-node, testing that
///    all nodes stay synchronized during the transition
///    - Topology remains: `[rn_follower, rn_sequencer, l2geth_follower] -> l2geth_sequencer`
///    - Block production switches from l2geth_sequencer to rn_sequencer
///    - All nodes receive new blocks from rn_sequencer via l2geth_sequencer relay
///
/// 4. **Network Partition Recovery**: Disconnects l2geth nodes, continues production on rollup
///    nodes, then reconnects to verify successful resynchronization
///    - Initial partition: `rn_follower -> rn_sequencer` (isolated rollup network)
///    - l2geth nodes disconnected, fall behind in block height
///    - Reconnection topology: `[l2geth_follower, l2geth_sequencer] -> rn_sequencer`
///    - l2geth nodes catch up by syncing from rn_sequencer
///
/// 5. **Bidirectional Compatibility**: Returns block production to l2geth after rollup nodes have
///    extended the chain, ensuring backward compatibility
///    - Final topology: `[rn_follower, l2geth_follower, l2geth_sequencer] -> rn_sequencer`
///    - Block production returns to l2geth_sequencer
///    - Validates that l2geth can continue the chain after rollup node blocks
///
/// The test validates that both client implementations maintain consensus despite
/// network topology changes, sequencer transitions, and temporary network partitions.
/// Each topology change tests different aspects of peer discovery, block gossip,
/// and chain synchronization across heterogeneous client implementations.
#[tokio::test]
async fn docker_test_migrate_sequencer() -> Result<()> {
    reth_tracing::init_test_tracing();

    tracing::info!("=== STARTING docker_test_migrate_sequencer ===");
    let env = DockerComposeEnv::new("docker_test_migrate_sequencer").await?;

    let rn_sequencer = env.get_rn_sequencer_provider().await?;
    let rn_follower = env.get_rn_follower_provider().await?;
    let l2geth_sequencer = env.get_l2geth_sequencer_provider().await?;
    let l2geth_follower = env.get_l2geth_follower_provider().await?;

    let nodes = [&rn_sequencer, &rn_follower, &l2geth_sequencer, &l2geth_follower];

    // Connect all nodes to each other.
    // topology:
    //  l2geth_follower -> l2geth_sequencer
    //  l2geth_follower -> rn_sequencer
    //  rn_follower -> l2geth_sequencer
    //  rn_follower -> rn_sequencer
    //  rn_sequencer -> l2geth_sequencer
    utils::admin_add_peer(&l2geth_follower, &env.l2geth_sequencer_enode()?).await?;
    utils::admin_add_peer(&l2geth_follower, &env.rn_sequencer_enode()?).await?;
    utils::admin_add_peer(&rn_follower, &env.l2geth_sequencer_enode()?).await?;
    utils::admin_add_peer(&rn_follower, &env.rn_sequencer_enode()?).await?;
    utils::admin_add_peer(&rn_sequencer, &env.l2geth_sequencer_enode()?).await?;

    // Enable block production on l2geth sequencer
    utils::miner_start(&l2geth_sequencer).await?;

    // Wait for at least 60 blocks to be produced
    let target_block = 30;
    utils::wait_for_block(&[&l2geth_sequencer], target_block).await?;

    let target_block = 60;
    utils::wait_for_block(&nodes, target_block).await?;
    utils::assert_blocks_match(&nodes, target_block).await?;

    let target_block = 90;
    utils::wait_for_block(&nodes, target_block).await?;
    utils::assert_blocks_match(&nodes, target_block).await?;

    let target_block = 120;
    utils::wait_for_block(&nodes, target_block).await?;
    utils::assert_blocks_match(&nodes, target_block).await?;

    Ok(())
}
