use alloy_provider::{Provider, ProviderBuilder};
use eyre::Result;
use scroll_alloy_network::Scroll;
use std::{fs, process::Command, time::Duration};

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

const RN_SEQUENCER_ENODE: &str= "enode://e7f7e271f62bd2b697add14e6987419758c97e83b0478bd948f5f2d271495728e7edef5bd78ad65258ac910f28e86928ead0c42ee51f2a0168d8ca23ba939766@{IP}:30303";
const L2GETH_SEQEUNCER_ENODE: &str = "enode://8fc4f6dfd0a2ebf56560d0b0ef5e60ad7bcb01e13f929eae53a4c77086d9c1e74eb8b8c8945035d25c6287afdd871f0d41b3fd7e189697decd0f13538d1ac620@{IP}:30303";

pub struct DockerComposeEnv {
    project_name: String,
    compose_file: String,
}

impl DockerComposeEnv {
    // ===== CONSTRUCTOR AND LIFECYCLE =====

    /// Create a new DockerComposeEnv and wait for all services to be ready
    pub async fn new(test_name: &str) -> Result<Self> {
        let project_name = format!("test-{test_name}");
        let compose_file = "docker-compose.test.yml";

        tracing::info!("🚀 Starting test environment: {project_name}");

        // Pre-cleanup existing containers to avoid conflicts
        Self::cleanup(&compose_file, &project_name, false);

        // Start the environment
        let env = Self::start_environment(compose_file, &project_name)?;

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
    fn cleanup(compose_file: &str, project_name: &str, dump_logs: bool) {
        tracing::info!("🧹 Cleaning up environment: {project_name}");

        if dump_logs {
            // Dump logs for all containers before cleanup
            Self::dump_container_logs_to_files(compose_file, project_name);
        }

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
                    // TODO: assert chain ID and genesis hash matches expected values (hardcoded
                    // constants)
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

    /// Get the IP address of a container within the project network
    fn get_container_ip(&self, container_name: &str) -> Result<String> {
        let output = Command::new("docker")
            .args([
                "inspect",
                "-f",
                "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}",
                container_name,
            ])
            .output()
            .map_err(|e| eyre::eyre!("Failed to run docker inspect: {}", e))?;

        if !output.status.success() {
            return Err(eyre::eyre!(
                "Failed to get container IP for {}: {}",
                container_name,
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let ip = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if ip.is_empty() {
            return Err(eyre::eyre!("No IP address found for container {}", container_name));
        }

        Ok(ip)
    }

    /// Get the rollup node sequencer enode URL with resolved IP address
    pub fn rn_sequencer_enode(&self) -> Result<String> {
        let ip = self.get_container_ip("rollup-node-sequencer")?;
        Ok(RN_SEQUENCER_ENODE.replace("{IP}", &ip))
    }

    /// Get the l2geth sequencer enode URL with resolved IP address
    pub fn l2geth_sequencer_enode(&self) -> Result<String> {
        let ip = self.get_container_ip("l2geth-sequencer")?;
        Ok(L2GETH_SEQEUNCER_ENODE.replace("{IP}", &ip))
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

    /// Dump logs for all containers to individual files
    fn dump_container_logs_to_files(compose_file: &str, project_name: &str) {
        tracing::debug!("📝 Dumping container logs to files for project: {project_name}");

        // Create logs directory
        let logs_dir = format!("target/test-logs/{project_name}");
        if let Err(e) = fs::create_dir_all(&logs_dir) {
            tracing::error!("Failed to create logs directory {logs_dir}: {e}");
            return;
        }

        // Get list of all containers for this project
        let containers_output = Command::new("docker")
            .args([
                "compose",
                "-f",
                compose_file,
                "-p",
                project_name,
                "ps",
                "--format",
                "{{.Name}}",
            ])
            .output();

        let container_names = match containers_output {
            Ok(output) => {
                if output.status.success() {
                    String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                } else {
                    tracing::error!(
                        "Failed to get container list: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    return;
                }
            }
            Err(e) => {
                tracing::error!("Failed to run docker compose ps: {e}");
                return;
            }
        };

        // Dump logs for each container
        for container_name in container_names {
            let log_file_path = format!("{logs_dir}/{container_name}.log");

            tracing::debug!("📋 Dumping logs for container '{container_name}' to {log_file_path}");

            let logs_output = Command::new("docker").args(["logs", &container_name]).output();

            match logs_output {
                Ok(logs) => {
                    let mut content = String::new();

                    // Combine stdout and stderr
                    let stdout = String::from_utf8_lossy(&logs.stdout);
                    let stderr = String::from_utf8_lossy(&logs.stderr);

                    if !stdout.trim().is_empty() {
                        content.push_str("=== STDOUT ===\n");
                        content.push_str(&stdout);
                        content.push('\n');
                    }

                    if !stderr.trim().is_empty() {
                        content.push_str("=== STDERR ===\n");
                        content.push_str(&stderr);
                        content.push('\n');
                    }

                    if let Err(e) = fs::write(&log_file_path, content) {
                        tracing::error!("Failed to write logs to {log_file_path}: {e}");
                    } else {
                        tracing::debug!("✅ Saved logs for '{container_name}' to {log_file_path}");
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to get logs for container '{container_name}': {e}");
                }
            }
        }

        tracing::info!("📁 All container logs dumped to: {logs_dir}");
    }
}

impl Drop for DockerComposeEnv {
    fn drop(&mut self) {
        Self::cleanup(&self.compose_file, &self.project_name, true);
    }
}
