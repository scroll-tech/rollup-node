use eyre::Result;
use std::sync::{atomic::AtomicBool, Arc};
use tests::*;

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

    // Start single continuous transaction sender for entire test
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();
    let rn_follower_clone = env.get_rn_follower_provider().await.unwrap();
    let l2geth_follower_clone = env.get_l2geth_follower_provider().await.unwrap();
    let tx_sender = tokio::spawn(async move {
        utils::run_continuous_tx_sender(stop_clone, &[&rn_follower_clone, &l2geth_follower_clone])
            .await
    });

    tracing::info!("🔄 Started continuous transaction sender for entire test");

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

    utils::stop_continuous_tx_sender(stop, tx_sender).await?;

    Ok(())
}
