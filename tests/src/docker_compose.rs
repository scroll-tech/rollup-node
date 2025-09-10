use alloy_provider::{Provider, ProviderBuilder};
use eyre::Result;
use scroll_alloy_network::Scroll;
use std::{
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// A wrapper that combines a provider with its human-readable name for testing
pub struct NamedProvider {
    pub provider: Box<dyn Provider<Scroll>>,
    pub name: &'static str,
}

impl NamedProvider {
    pub fn new(provider: Box<dyn Provider<Scroll>>, name: &'static str) -> Self {
        Self { provider, name }
    }
}

/// The sequencer node RPC URL for the Docker Compose environment.
const RN_SEQUENCER_RPC_URL: &str = "http://localhost:8545";

/// The follower node RPC URL for the Docker Compose environment.
const RN_FOLLOWER_RPC_URL: &str = "http://localhost:8546";

/// The l2geth node RPC URL for the Docker Compose environment.
const L2GETH_SEQUENCER_RPC_URL: &str = "http://localhost:8547";

const L2GETH_FOLLOWER_RPC_URL: &str = "http://localhost:8548";

pub struct DockerComposeEnv {
    project_name: String,
    compose_file: String,
}

impl DockerComposeEnv {
    // ===== CONSTRUCTOR AND LIFECYCLE =====

    /// Create a new DockerComposeEnv and wait for all services to be ready
    pub async fn new(test_name: &str) -> Result<Self> {
        let start = SystemTime::now();
        let since_the_epoch = start.duration_since(UNIX_EPOCH).expect("Time went backwards");
        let timestamp = since_the_epoch.as_secs();
        let project_name = format!("test-{test_name}-{timestamp}");
        let compose_file = "docker-compose.test.yml".to_string();

        tracing::info!("🚀 Starting test environment: {project_name}");

        // Pre-cleanup existing containers to avoid conflicts
        Self::cleanup(&compose_file, &project_name);

        // Start the environment
        let env = Self::start_environment(&compose_file, &project_name)?;

        // Wait for all services to be ready
        tracing::info!("⏳ Waiting for services to be ready...");
        Self::wait_for_l2_node_ready(RN_SEQUENCER_RPC_URL, 30).await?;
        Self::wait_for_l2_node_ready(RN_FOLLOWER_RPC_URL, 30).await?;
        Self::wait_for_l2_node_ready(L2GETH_SEQUENCER_RPC_URL, 30).await?;
        Self::wait_for_l2_node_ready(L2GETH_FOLLOWER_RPC_URL, 30).await?;

        tracing::info!("✅ All services are ready!");
        Ok(env)
    }

    fn start_environment(compose_file: &str, project_name: &str) -> Result<Self> {
        tracing::info!("📦 Starting docker-compose services...");

        let mut child = Command::new("docker")
            .args([
                "compose",
                "-f",
                compose_file,
                "-p",
                project_name,
                "up",
                "-d",
                // "--force-recreate",
                // "--build",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to start docker-compose");

        if let Some(stdout) = child.stdout.take() {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                tracing::debug!("📦 Docker: {line}");
            }
        }

        let output = child.wait_with_output().expect("Failed to wait for docker-compose");

        if !output.status.success() {
            tracing::error!("Docker-compose stderr: {}", String::from_utf8_lossy(&output.stderr));

            // Show logs for debugging before returning error
            Self::show_all_container_logs(compose_file, project_name);

            return Err(eyre::eyre!("Failed to spin up docker-compose"));
        }

        Ok(Self { project_name: project_name.to_string(), compose_file: compose_file.to_string() })
    }

    /// Cleanup the environment
    fn cleanup(compose_file: &str, project_name: &str) {
        tracing::info!("🧹 Cleaning up environment: {project_name}");

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
                "3",
            ])
            .output();

        let _result = Command::new("docker")
            .args(["compose", "-f", compose_file, "rm", "--force", "--volumes", "--stop"])
            .output();

        tracing::info!("✅ Cleanup completed");
    }

    // ===== PROVIDER FACTORIES =====

    /// Get a configured sequencer provider
    pub async fn get_rn_sequencer_provider(&self) -> Result<NamedProvider> {
        let provider = ProviderBuilder::<_, _, Scroll>::default()
            .with_recommended_fillers()
            .connect(RN_SEQUENCER_RPC_URL)
            .await
            .map_err(|e| eyre::eyre!("Failed to connect to RN sequencer: {}", e))?;
        Ok(NamedProvider::new(Box::new(provider), "RN Sequencer"))
    }

    /// Get a configured follower provider
    pub async fn get_rn_follower_provider(&self) -> Result<NamedProvider> {
        let provider = ProviderBuilder::<_, _, Scroll>::default()
            .with_recommended_fillers()
            .connect(RN_FOLLOWER_RPC_URL)
            .await
            .map_err(|e| eyre::eyre!("Failed to connect to RN follower: {}", e))?;
        Ok(NamedProvider::new(Box::new(provider), "RN Follower"))
    }

    /// Get a configured l2geth sequencer provider
    pub async fn get_l2geth_sequencer_provider(&self) -> Result<NamedProvider> {
        let provider = ProviderBuilder::<_, _, Scroll>::default()
            .with_recommended_fillers()
            .connect(L2GETH_SEQUENCER_RPC_URL)
            .await
            .map_err(|e| eyre::eyre!("Failed to connect to l2geth sequencer: {}", e))?;
        Ok(NamedProvider::new(Box::new(provider), "L2Geth Sequencer"))
    }

    /// Get a configured l2geth follower provider
    pub async fn get_l2geth_follower_provider(&self) -> Result<NamedProvider> {
        let provider = ProviderBuilder::<_, _, Scroll>::default()
            .with_recommended_fillers()
            .connect(L2GETH_FOLLOWER_RPC_URL)
            .await
            .map_err(|e| eyre::eyre!("Failed to connect to l2geth follower: {}", e))?;
        Ok(NamedProvider::new(Box::new(provider), "L2Geth Follower"))
    }

    // ===== UTILITIES =====

    /// Wait for L2 node to be ready
    async fn wait_for_l2_node_ready(provider_url: &str, max_retries: u32) -> Result<()> {
        for i in 0..max_retries {
            match ProviderBuilder::<_, _, Scroll>::default()
                .with_recommended_fillers()
                .connect(provider_url)
                .await
            {
                Ok(provider) => match provider.get_chain_id().await {
                    Ok(chain_id) => {
                        tracing::info!("✅ L2 node ready - Chain ID: {chain_id}");
                        return Ok(());
                    }
                    Err(e) => {
                        let attempt = i + 1;
                        tracing::warn!(
                            "⏳ L2 node not ready yet (attempt {attempt}/{max_retries}): {e}"
                        );
                    }
                },
                Err(e) => {
                    let attempt = i + 1;
                    tracing::warn!("⏳ Waiting for L2 node (attempt {attempt}/{max_retries}): {e}");
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        eyre::bail!(
            "L2 node failed to become ready after {max_retries} attempts, url: {provider_url}"
        );
    }

    /// Show logs for all containers
    fn show_all_container_logs(compose_file: &str, project_name: &str) {
        tracing::info!("🔍 Getting all container logs...");

        let logs_output = Command::new("docker")
            .args(["compose", "-f", compose_file, "-p", project_name, "logs"])
            .output();

        if let Ok(logs) = logs_output {
            let stdout = String::from_utf8_lossy(&logs.stdout);
            let stderr = String::from_utf8_lossy(&logs.stderr);
            if !stdout.trim().is_empty() {
                tracing::error!("❌ All container logs (stdout):\n{stdout}");
            }
            if !stderr.trim().is_empty() {
                tracing::error!("❌ All container logs (stderr):\n{stderr}");
            }
        }
    }
}

impl Drop for DockerComposeEnv {
    fn drop(&mut self) {
        Self::cleanup(&self.compose_file, &self.project_name);
    }
}
