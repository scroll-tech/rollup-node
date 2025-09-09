//! Tests for block propagation to both geth and reth follower nodes.

use eyre::Result;
use tests::*;

#[tokio::test]
async fn test_docker_block_propagation_to_both_clients() -> Result<()> {
    reth_tracing::init_test_tracing();

    tracing::info!("=== STARTING test_docker_block_propagation_to_both_clients ===");
    let env = DockerComposeEnv::new("multi-client-propagation").await?;

    let rn_sequencer = env.get_rn_sequencer_provider().await?;
    let rn_follower = env.get_rn_follower_provider().await?;
    let l2geth_sequencer = env.get_l2geth_sequencer_provider().await?;
    let l2geth_follower = env.get_l2geth_follower_provider().await?;

    let nodes = [&rn_sequencer, &rn_follower, &l2geth_sequencer, &l2geth_follower];

    // TODO: Enable block production on l2geth sequencer
    utils::miner_start(&l2geth_sequencer).await?;

    // Wait for all nodes to be at block 10
    let target_block = 10;
    utils::wait_for_block(&nodes, target_block).await?;
    // Verify blocks match across all clients
    utils::assert_blocks_match(&nodes, target_block).await?;

    Ok(())
}
