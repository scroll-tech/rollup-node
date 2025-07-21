use super::common::{DockerComposeEnv, TestClients};
use std::time::Duration;

#[tokio::test]
async fn test_basic_node_sync() {
    println!("=== STARTING test_basic_node_sync ===");
    let _env = DockerComposeEnv::new("basic-node-sync");

    // Wait a bit for services to initialize
    println!("⏳ Waiting 5 seconds for services to fully initialize...");
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Create clients with retry/error handling
    let clients = match retry_operation(
        || async {
            TestClients::new(&_env.get_sequencer_rpc_url(), &_env.get_follower_rpc_url()).await
        },
        3,
    )
    .await
    {
        Ok(clients) => {
            println!("✅ Test clients created successfully");
            clients
        }
        Err(e) => {
            println!("❌ Failed to create test clients: {}", e);
            _env.show_container_logs("rollup-node-sequencer");
            _env.show_container_logs("rollup-node-follower");
            panic!("Failed to create test clients: {}", e);
        }
    };

    // Verify connectivity with retry/detailed logging
    match retry_operation(|| async { clients.verify_connectivity().await }, 3).await {
        Ok(_) => println!("✅ Connectivity verification passed"),
        Err(e) => {
            println!("❌ Connectivity verification failed: {}", e);
            _env.show_container_logs("rollup-node-sequencer");
            _env.show_container_logs("rollup-node-follower");
            panic!("Failed to verify connectivity: {}", e);
        }
    }

    // Sequencer produces 5 blocks
    let target_block =
        clients.wait_for_sequencer_blocks(5).await.expect("Sequencer should produce blocks");

    // Follower syncs to target
    clients.wait_for_follower_sync(target_block).await.expect("Follower should sync to sequencer");

    // Verify blocks match
    for block_num in 1..=target_block {
        clients
            .verify_blocks_match(block_num)
            .await
            .unwrap_or_else(|e| panic!("Block {} mismatch: {}", block_num, e));
    }

    println!("✅ Basic node sync test completed successfully!");
}

/// Helper function to retry operations with exponential backoff
async fn retry_operation<F, Fut, T, E>(mut operation: F, max_attempts: usize) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut last_error = None;

    for attempt in 1..=max_attempts {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                println!("❌ Attempt {} failed: {}", attempt, e);
                last_error = Some(e);

                if attempt < max_attempts {
                    let delay = Duration::from_secs(attempt as u64 * 2);
                    println!("⏳ Waiting {:?} before retry...", delay);
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    Err(last_error.unwrap())
}
