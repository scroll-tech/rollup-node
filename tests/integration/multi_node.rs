use super::common::{DockerComposeEnv, TestClients};

#[tokio::test]
async fn test_basic_node_sync() {
    println!("=== STARTING test_basic_node_sync ===");
    let _env = DockerComposeEnv::new("basic-node-sync");

    let clients = TestClients::new(&_env.get_sequencer_rpc_url(), &_env.get_follower_rpc_url())
        .await
        .expect("Failed to create test clients");

    // Verify connectivity
    clients.verify_connectivity().await.expect("Failed to verify connectivity");

    // Wait for sequencer to produce 5 blocks
    let target_block =
        clients.wait_for_sequencer_blocks(5).await.expect("Sequencer should produce blocks");

    // Wait for follower to sync
    clients.wait_for_follower_sync(target_block).await.expect("Follower should sync to sequencer");

    // Verify blocks match
    for block_num in 1..=target_block {
        clients
            .verify_blocks_match(block_num)
            .await
            .unwrap_or_else(|e| panic!("Block {} mismatch: {}", block_num, e));
    }
}
