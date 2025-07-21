use super::common::{DockerComposeEnv, TestClients};
use std::time::Duration;

#[tokio::test]
async fn test_basic_block_production() {
    println!("🚀 Starting basic block production test");

    let _env = DockerComposeEnv::new("basic-block-production");

    // Wait a bit longer for services to fully initialize
    println!("⏳ Waiting 5 seconds for services to fully initialize...");
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Create test clients with enhanced error handling
    let clients =
        match TestClients::new(&_env.get_sequencer_rpc_url(), &_env.get_follower_rpc_url()).await {
            Ok(clients) => {
                println!("✅ Test clients created successfully");
                clients
            }
            Err(e) => {
                println!("❌ Failed to create test clients: {}", e);
                println!("🔍 Getting container logs for debugging...");
                _env.show_container_logs("rollup-node-sequencer");
                _env.show_container_logs("rollup-node-follower");
                panic!("Failed to create test clients: {}", e);
            }
        };

    // Verify basic connectivity with detailed error reporting
    match clients.verify_connectivity().await {
        Ok(_) => println!("✅ Connectivity verification passed"),
        Err(e) => {
            println!("❌ Connectivity verification failed: {}", e);
            println!("🔍 Getting detailed container logs...");
            _env.show_container_logs("rollup-node-sequencer");
            _env.show_container_logs("rollup-node-follower");

            // Check port status
            check_port_status(8545, "sequencer");
            check_port_status(8547, "follower");

            panic!("Failed to verify connectivity: {}", e);
        }
    }

    // Get initial block number with retry mechanism
    let initial_block =
        match retry_operation(|| async { clients.l2_provider().get_block_number().await }, 3).await
        {
            Ok(block) => {
                println!("📊 Initial L2 block: {}", block);
                block
            }
            Err(e) => {
                println!("❌ Failed to get initial block number: {}", e);
                _env.show_container_logs("rollup-node-sequencer");
                panic!("Failed to get initial L2 block number: {}", e);
            }
        };

    // Wait for at least 5 new blocks to be produced (increased timeout)
    println!("⏳ Waiting for 5 new blocks to be produced...");
    let final_block = match clients.wait_for_l2_blocks(5, 60).await {
        Ok(block) => {
            println!("📊 Final L2 block: {}", block);
            block
        }
        Err(e) => {
            println!("❌ Failed to wait for new blocks: {}", e);

            // Get current block to see if any progress was made
            if let Ok(current_block) = clients.l2_provider().get_block_number().await {
                println!("📊 Current block when timeout occurred: {}", current_block);
                println!("📊 Blocks produced: {}", current_block.saturating_sub(initial_block));
            }

            println!("🔍 Getting container logs for block production failure...");
            _env.show_container_logs("rollup-node-sequencer");
            _env.show_container_logs("rollup-node-follower");

            panic!("L2 should produce new blocks within 60 seconds: {}", e);
        }
    };

    // Verify that enough blocks were actually produced
    let blocks_produced = final_block.saturating_sub(initial_block);
    println!("📊 Total blocks produced: {}", blocks_produced);

    assert!(
        final_block >= initial_block + 5,
        "L2 should have produced at least 5 blocks: initial={}, final={}, produced={}",
        initial_block,
        final_block,
        blocks_produced
    );

    println!("✅ Basic block production test completed successfully!");
    println!(
        "📊 Test summary: produced {} blocks from {} to {}",
        blocks_produced, initial_block, final_block
    );
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

/// Helper function to check port status for debugging
fn check_port_status(port: u16, service_name: &str) {
    println!("🔍 Checking {} port {} status...", service_name, port);

    // Use lsof to check what's listening on the port
    let lsof_output =
        std::process::Command::new("lsof").args(&["-i", &format!(":{}", port)]).output();

    if let Ok(output) = lsof_output {
        let result = String::from_utf8_lossy(&output.stdout);
        if !result.trim().is_empty() {
            println!("🔍 {} port {} status:\n{}", service_name, port, result);
        } else {
            println!("⚠️ No process found listening on {} port {}", service_name, port);
        }
    } else {
        println!("⚠️ Could not check {} port {} status (lsof failed)", service_name, port);
    }

    // Also try netstat as backup
    let netstat_output = std::process::Command::new("netstat").args(&["-an"]).output();

    if let Ok(output) = netstat_output {
        let result = String::from_utf8_lossy(&output.stdout);
        let port_str = format!(":{}", port);
        let listening_lines: Vec<&str> = result
            .lines()
            .filter(|line| line.contains(&port_str) && line.contains("LISTEN"))
            .collect();

        if !listening_lines.is_empty() {
            println!("🔍 Netstat shows {} port {} is listening:", service_name, port);
            for line in listening_lines {
                println!("  {}", line);
            }
        } else {
            println!("⚠️ Netstat shows {} port {} is not listening", service_name, port);
        }
    }
}
