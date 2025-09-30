use eyre::Result;
use std::sync::{atomic::AtomicBool, Arc};
use tests::*;

/// Tests cross-client block propagation and synchronization between heterogeneous nodes.
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
async fn docker_test_heterogeneous_client_sync_and_sequencer_handoff() -> Result<()> {
    reth_tracing::init_test_tracing();

    tracing::info!("=== STARTING docker_test_heterogeneous_client_sync_and_sequencer_handoff ===");
    let env = DockerComposeEnv::new("docker_test_heterogeneous_client_sync_and_sequencer_handoff")
        .await?;

    let rn_sequencer = env.get_rn_sequencer_provider().await?;
    let rn_follower = env.get_rn_follower_provider().await?;
    let l2geth_sequencer = env.get_l2geth_sequencer_provider().await?;
    let l2geth_follower = env.get_l2geth_follower_provider().await?;

    let rn_nodes = [&rn_sequencer, &rn_follower];
    let l2geth_nodes = [&l2geth_sequencer, &l2geth_follower];
    let nodes = [&rn_sequencer, &rn_follower, &l2geth_sequencer, &l2geth_follower];

    // Connect only l2geth nodes first
    // l2geth_follower -> l2geth_sequencer
    utils::admin_add_peer(&l2geth_follower, &env.l2geth_sequencer_enode()?).await?;
    tracing::info!("✅ Connected l2geth follower to l2geth sequencer");

    // Enable block production on l2geth sequencer
    utils::miner_start(&l2geth_sequencer).await?;

    // Start single continuous transaction sender for entire test
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();
    let rn_follower_clone = env.get_rn_follower_provider().await.unwrap();
    let l2geth_follower_clone = env.get_l2geth_follower_provider().await.unwrap();
    let tx_sender = tokio::spawn(async move {
        utils::run_continuous_tx_sender(stop_clone, &[&rn_follower_clone, &l2geth_follower_clone])
            .await
    });
    let stop_clone = stop.clone();
    let l1_message_sender =
        tokio::spawn(async move { utils::run_continuous_l1_message_sender(stop_clone).await });

    tracing::info!("🔄 Started continuous L1 message and L2 transaction sender for entire test");

    // Wait for at least 10 blocks to be produced
    let target_block = 10;
    utils::wait_for_block(&[&l2geth_sequencer], target_block).await?;
    utils::miner_stop(&l2geth_sequencer).await?;

    let latest_block = l2geth_sequencer.get_block_number().await?;

    // Wait for all l2geth nodes to reach the latest block
    utils::wait_for_block(&l2geth_nodes, latest_block).await?;
    utils::assert_blocks_match(&l2geth_nodes, latest_block).await?;
    tracing::info!("✅ All l2geth nodes reached block {}", latest_block);

    // Assert rollup nodes are still at block 0
    utils::assert_latest_block(&rn_nodes, 0).await?;

    // Connect rollup nodes to l2geth sequencer
    // topology:
    //  l2geth_follower -> l2geth_sequencer
    //  rn_follower -> l2geth_sequencer
    //  rn_sequencer -> l2geth_sequencer
    utils::admin_add_peer(&rn_follower, &env.l2geth_sequencer_enode()?).await?;
    utils::admin_add_peer(&rn_sequencer, &env.l2geth_sequencer_enode()?).await?;
    tracing::info!("✅ Connected rollup nodes to l2geth sequencer");

    // Continue block production on l2geth sequencer
    utils::miner_start(&l2geth_sequencer).await?;

    // Wait for all nodes to reach target block
    let target_block = latest_block + 10;
    utils::wait_for_block(&nodes, target_block).await?;

    utils::miner_stop(&l2geth_sequencer).await?;
    let latest_block = l2geth_sequencer.get_block_number().await?;
    utils::wait_for_block(&nodes, latest_block).await?;
    utils::assert_blocks_match(&nodes, latest_block).await?;
    tracing::info!("✅ All nodes reached block {}", latest_block);

    // Enable sequencing on RN sequencer
    tracing::info!("Enabling sequencing on RN sequencer");
    utils::enable_automatic_sequencing(&rn_sequencer).await?;
    let target_block = latest_block + 10;
    utils::wait_for_block(&nodes, target_block).await?;

    utils::disable_automatic_sequencing(&rn_sequencer).await?;
    let latest_block = rn_sequencer.get_block_number().await?;
    utils::wait_for_block(&nodes, latest_block).await?;
    utils::assert_blocks_match(&nodes, latest_block).await?;
    tracing::info!("✅ All nodes reached block {}", latest_block);

    // Disconnect l2geth follower from l2geth sequencer
    // topology:
    //  rn_follower -> rn_sequencer
    utils::admin_remove_peer(&rn_follower, &env.l2geth_sequencer_enode()?).await?;
    utils::admin_remove_peer(&rn_sequencer, &env.l2geth_sequencer_enode()?).await?;
    utils::admin_remove_peer(&l2geth_follower, &env.l2geth_sequencer_enode()?).await?;
    utils::admin_add_peer(&rn_follower, &env.rn_sequencer_enode()?).await?;

    // Continue sequencing on RN sequencer for at least 10 blocks
    utils::enable_automatic_sequencing(&rn_sequencer).await?;
    let target_block = latest_block + 10;
    utils::wait_for_block(&rn_nodes, target_block).await?;

    // Make sure l2geth nodes are still at the old block -> they need to sync once reconnected
    assert!(
        l2geth_sequencer.get_block_number().await? <= target_block + 1,
        "l2geth sequencer should be at most at block {}, but is at {}",
        target_block + 1,
        l2geth_sequencer.get_block_number().await?
    );
    assert!(
        l2geth_follower.get_block_number().await? <= target_block + 1,
        "l2geth follower should be at most at block {}, but is at {}",
        target_block + 1,
        l2geth_follower.get_block_number().await?
    );

    // Reconnect l2geth follower to l2geth sequencer and let them sync
    // topology:
    //  rn_follower -> rn_sequencer
    //  l2geth_follower -> rn_sequencer
    //  l2geth_sequencer -> rn_sequencer
    utils::admin_add_peer(&l2geth_follower, &env.rn_sequencer_enode()?).await?;
    utils::admin_add_peer(&l2geth_sequencer, &env.rn_sequencer_enode()?).await?;

    // Wait for all nodes to reach the same block again
    let target_block = target_block + 10;
    utils::wait_for_block(&nodes, target_block).await?;
    tracing::info!("✅ l2geth nodes synced to block {}", target_block);

    // Disable sequencing on RN sequencer
    utils::disable_automatic_sequencing(&rn_sequencer).await?;
    let latest_block = rn_sequencer.get_block_number().await?;
    tracing::info!("Switched RN sequencing off at block {}", latest_block);
    utils::wait_for_block(&nodes, latest_block).await?;
    utils::assert_blocks_match(&nodes, latest_block).await?;

    // start sequencing on l2geth sequencer again and make sure all nodes reach the same block in
    // the end
    utils::miner_start(&l2geth_sequencer).await?;
    let target_block = latest_block + 20;
    utils::wait_for_block(&nodes, target_block).await?;
    assert_blocks_match(&nodes, target_block).await?;

    utils::stop_continuous_tx_sender(stop.clone(), tx_sender).await?;
    utils::stop_continuous_l1_message_sender(stop, l1_message_sender).await?;

    // Make sure l1 message queue is processed on all l2geth nodes
    let q = utils::get_l1_message_index_at_finalized().await?;
    utils::wait_for_l1_message_queue_index_reached(&[&l2geth_sequencer, &l2geth_follower], q)
        .await?;

    Ok(())
}
