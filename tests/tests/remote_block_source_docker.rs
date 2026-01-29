use eyre::Result;
use tests::*;

#[tokio::test]
async fn docker_test_remote_block_source_basic() -> Result<()> {
    reth_tracing::init_test_tracing();

    tracing::info!("=== STARTING docker_test_remote_block_source_basic ===");
    let env = DockerComposeEnv::new_remote_source("docker_test_remote_block_source_basic").await?;

    let rn_sequencer = env.get_rn_sequencer_provider().await?;
    let rn_remote_source = env.get_rn_remote_source_provider().await?;
    let l2geth = env.get_l2geth_skipsignercheck_provider().await?;

    utils::enable_automatic_sequencing(&rn_sequencer).await?;

    let target_block = 10;
    utils::wait_for_block(&[&rn_sequencer], target_block).await?;
    let sequencer_block = rn_sequencer.get_block_number().await?;

    utils::wait_for_block(&[&rn_remote_source], sequencer_block).await?;
    utils::assert_blocks_match(&[&rn_sequencer, &rn_remote_source], sequencer_block).await?;

    utils::admin_add_peer(&l2geth, &env.rn_remote_source_enode()?).await?;
    utils::wait_for_block(&[&l2geth], sequencer_block).await?;
    utils::assert_blocks_match(&[&rn_remote_source, &l2geth], sequencer_block).await?;

    let remote_block = rn_remote_source.get_block_number().await?;
    assert!(
        remote_block >= sequencer_block,
        "remote source should be at or ahead of sequencer: remote={}, sequencer={}",
        remote_block,
        sequencer_block
    );

    Ok(())
}

#[tokio::test]
async fn docker_test_remote_block_source_recovery() -> Result<()> {
    reth_tracing::init_test_tracing();

    tracing::info!("=== STARTING docker_test_remote_block_source_recovery ===");
    let env = DockerComposeEnv::new_remote_source("docker_test_remote_block_source_recovery").await?;

    let rn_sequencer = env.get_rn_sequencer_provider().await?;
    let rn_remote_source = env.get_rn_remote_source_provider().await?;

    utils::enable_automatic_sequencing(&rn_sequencer).await?;

    let target_block = 10;
    utils::wait_for_block(&[&rn_sequencer], target_block).await?;
    let sequencer_block = rn_sequencer.get_block_number().await?;
    utils::wait_for_block(&[&rn_remote_source], sequencer_block).await?;

    tracing::info!("Restarting rollup-node-sequencer container");
    env.restart_container(&rn_sequencer).await?;
    let rn_sequencer = env.get_rn_sequencer_provider().await?;
    utils::enable_automatic_sequencing(&rn_sequencer).await?;

    let target_block = sequencer_block + 5;
    utils::wait_for_block(&[&rn_sequencer], target_block).await?;
    utils::wait_for_block(&[&rn_remote_source], target_block).await?;
    utils::assert_blocks_match(&[&rn_sequencer, &rn_remote_source], target_block).await?;

    Ok(())
}
