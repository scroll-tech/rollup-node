use eyre::Result;
use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
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
    let stop_clone = stop.clone();
    let l1_message_sender =
        tokio::spawn(async move { utils::run_continuous_l1_message_sender(stop_clone).await });

    tracing::info!("🔄 Started continuous L1 message and L2 transaction sender for entire test");

    let enable_l2geth_sequencing = async || -> Result<()> {
        utils::disable_automatic_sequencing(&rn_sequencer).await?;
        let latest_block = rn_sequencer.get_block_number().await?;
        utils::wait_for_block(&nodes, latest_block).await?;

        tracing::info!("Enabling sequencing on l2geth sequencer");
        utils::miner_start(&l2geth_sequencer).await?;

        Ok(())
    };

    let enable_rn_sequencing = async || -> Result<()> {
        utils::miner_stop(&l2geth_sequencer).await?;
        let latest_block = l2geth_sequencer.get_block_number().await?;
        utils::wait_for_block(&nodes, latest_block).await?;

        tracing::info!("Enabling sequencing on RN sequencer");
        utils::enable_automatic_sequencing(&rn_sequencer).await?;

        Ok(())
    };

    // Alternate sequencing between l2geth and rn sequencer every 5 seconds. With a block time of
    // 500ms we should produce about 10 blocks each handoff.
    let latest_block = 0u64;
    loop {
        enable_l2geth_sequencing().await?;
        tokio::time::sleep(Duration::from_secs(5)).await;
        enable_rn_sequencing().await?;
        tokio::time::sleep(Duration::from_secs(5)).await;

        let latest_block = rn_sequencer.get_block_number().await?;
        if latest_block >= 120 {
            break;
        }
    }

    utils::wait_for_block(&nodes, latest_block).await?;
    utils::assert_blocks_match(&nodes, latest_block).await?;

    utils::stop_continuous_tx_sender(stop.clone(), tx_sender).await?;
    utils::stop_continuous_l1_message_sender(stop, l1_message_sender).await?;

    // Make sure l1 message queue is processed on all l2geth nodes
    let q = utils::get_l1_message_index_at_finalized().await?;
    utils::wait_for_l1_message_queue_index_reached(&[&l2geth_sequencer, &l2geth_follower], q)
        .await?;

    Ok(())
}
