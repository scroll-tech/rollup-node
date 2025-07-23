use alloy_provider::{Provider, ProviderBuilder};
use eyre::Result;
use scroll_alloy_network::Scroll;
use std::{
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub struct DockerComposeEnv {
    project_name: String,
    compose_file: String,
}

impl DockerComposeEnv {
    pub fn new(test_name: &str) -> Self {
        let start = SystemTime::now();
        let since_the_epoch = start.duration_since(UNIX_EPOCH).expect("Time went backwards");
        let timestamp = since_the_epoch.as_secs();
        let project_name = format!("test-{test_name}-{timestamp}");
        let compose_file = "docker-compose.test.yml".to_string();

        println!("🚀 Starting test environment: {project_name}");

        // Pre-cleanup existing containers to avoid conflicts
        Self::cleanup(&compose_file, &project_name);

        // Start the environment
        Self::start_environment(&compose_file, &project_name)
    }

    fn start_environment(compose_file: &str, project_name: &str) -> Self {
        println!("📦 Starting docker-compose services...");

        let mut child = Command::new("docker")
            .args([
                "compose",
                "-f",
                compose_file,
                "-p",
                project_name,
                "up",
                "-d",
                "--force-recreate",
                "--build",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to start docker-compose");

        if let Some(stdout) = child.stdout.take() {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                println!("📦 Docker: {line}");
            }
        }

        let output = child.wait_with_output().expect("Failed to wait for docker-compose");

        if !output.status.success() {
            eprintln!("Docker-compose stderr: {}", String::from_utf8_lossy(&output.stderr));

            // Show logs for debugging before panicking
            Self::show_all_container_logs(compose_file, project_name);

            panic!("Failed to spin up docker-compose");
        }

        Self { project_name: project_name.to_string(), compose_file: compose_file.to_string() }
    }

    // Wait for L2 node to be ready
    pub async fn wait_for_l2_node_ready(provider_url: &str, max_retries: u32) -> Result<()> {
        for i in 0..max_retries {
            match ProviderBuilder::<_, _, Scroll>::default()
                .with_recommended_fillers()
                .connect(provider_url)
                .await
            {
                Ok(provider) => match provider.get_chain_id().await {
                    Ok(chain_id) => {
                        println!("✅ L2 node ready - Chain ID: {chain_id}");
                        return Ok(());
                    }
                    Err(e) => {
                        let attempt = i + 1;
                        println!("⏳ L2 node not ready yet (attempt {attempt}/{max_retries}): {e}");
                    }
                },
                Err(e) => {
                    let attempt = i + 1;
                    println!("⏳ Waiting for L2 node (attempt {attempt}/{max_retries}): {e}");
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        eyre::bail!("L2 node failed to become ready after {max_retries} attempts")
    }

    /// Show logs for all containers
    fn show_all_container_logs(compose_file: &str, project_name: &str) {
        println!("🔍 Getting all container logs...");

        let logs_output = Command::new("docker")
            .args(["compose", "-f", compose_file, "-p", project_name, "logs"])
            .output();

        if let Ok(logs) = logs_output {
            let stdout = String::from_utf8_lossy(&logs.stdout);
            let stderr = String::from_utf8_lossy(&logs.stderr);
            if !stdout.trim().is_empty() {
                eprintln!("❌ All container logs (stdout):\n{stdout}");
            }
            if !stderr.trim().is_empty() {
                eprintln!("❌ All container logs (stderr):\n{stderr}");
            }
        }
    }

    /// Get Sequencer RPC URL
    pub fn get_sequencer_rpc_url(&self) -> String {
        "http://localhost:8545".to_string()
    }

    /// Get Follower RPC URL
    pub fn get_follower_rpc_url(&self) -> String {
        "http://localhost:8547".to_string()
    }

    /// Cleanup the environment
    fn cleanup(compose_file: &str, project_name: &str) {
        println!("🧹 Cleaning up environment: {project_name}");

        let _result = Command::new("docker")
            .args([
                "compose",
                "-f",
                compose_file,
                "-p",
                project_name,
                "down",
                "--volumes",
                "--remove-orphans",
                "--timeout",
                "30",
            ])
            .output();

        println!("✅ Cleanup completed");
    }
}

impl Drop for DockerComposeEnv {
    fn drop(&mut self) {
        Self::cleanup(&self.compose_file, &self.project_name);
    }
}
