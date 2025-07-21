use std::{process::Command, thread, time::Duration};
use uuid::Uuid;

pub struct DockerComposeEnv {
    project_name: String,
    compose_file: String,
    cleanup_on_drop: bool,
}

impl DockerComposeEnv {
    pub fn new(test_name: &str) -> Self {
        let uuid = Uuid::new_v4();
        let uuid_str = uuid.to_string();
        let project_name = format!("test-{}-{}", test_name, &uuid_str[..8]);
        let compose_file = "docker-compose.test.yml".to_string();

        println!("🚀 Starting test environment: {}", project_name);

        // Pre-cleanup existing containers to avoid conflicts
        Self::cleanup_existing_containers();

        // Start the environment
        let env = Self::start_environment(&compose_file, &project_name);
        env
    }

    /// Clean up any existing containers with the same names to avoid conflicts
    fn cleanup_existing_containers() {
        println!("🧹 Pre-cleaning existing containers...");

        let containers = ["rollup-node-sequencer", "rollup-node-follower"];
        for container in &containers {
            // Stop container if running
            let _ = Command::new("docker").args(&["stop", container]).output();

            // Remove container forcefully
            let _ = Command::new("docker").args(&["rm", "-f", container]).output();
        }

        // Clean up orphaned networks
        let _ = Command::new("docker").args(&["network", "prune", "-f"]).output();
    }

    fn start_environment(compose_file: &str, project_name: &str) -> Self {
        println!("📦 Starting docker-compose services...");

        let mut child = Command::new("docker-compose")
            .args(&[
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
            for line in reader.lines() {
                if let Ok(line) = line {
                    println!("📦 Docker: {}", line);
                }
            }
        }

        let output = child.wait_with_output().expect("Failed to wait for docker-compose");

        if !output.status.success() {
            eprintln!("Docker-compose stderr: {}", String::from_utf8_lossy(&output.stderr));

            // Show logs for debugging before panicking
            Self::show_all_container_logs(compose_file, project_name);

            panic!("Failed to spin up docker-compose");
        }

        let env = Self {
            project_name: project_name.to_string(),
            compose_file: compose_file.to_string(),
            cleanup_on_drop: true,
        };

        // Wait for services to be ready (with improved logic)
        env.wait_for_services();

        env
    }

    fn wait_for_services(&self) {
        println!("⏳ Waiting for services to be ready...");

        // First check if containers are running (without health checks)
        self.wait_for_container_running("rollup-node-sequencer");
        self.wait_for_container_running("rollup-node-follower");

        println!("✅ All services are running!");
    }

    /// Wait for container to be in running state
    fn wait_for_container_running(&self, container_name: &str) {
        for attempt in 1..=30 {
            // Check if container is running
            let output = Command::new("docker")
                .args(&["inspect", "--format={{.State.Running}}", container_name])
                .output();

            if let Ok(output) = output {
                let is_running = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if is_running == "true" {
                    println!("✅ {} is running", container_name);

                    // Additional check: try to connect to the port
                    if self.check_port_accessibility(container_name) {
                        return;
                    }
                } else if is_running == "false" {
                    // Container stopped, get exit code and logs
                    let exit_code_output = Command::new("docker")
                        .args(&["inspect", "--format={{.State.ExitCode}}", container_name])
                        .output();

                    if let Ok(exit_output) = exit_code_output {
                        let exit_code_str = String::from_utf8_lossy(&exit_output.stdout);
                        let exit_code = exit_code_str.trim();
                        eprintln!("❌ {} exited with code: {}", container_name, exit_code);
                    }

                    self.show_container_logs(container_name);
                    panic!("❌ {} failed to start (container exited)", container_name);
                }
            }

            if attempt % 5 == 0 {
                println!("⏳ Waiting for {}... (attempt {}/30)", container_name, attempt);
                // Show recent logs every 5 attempts
                self.show_recent_logs(container_name);
            }

            thread::sleep(Duration::from_secs(2));
        }

        // Final attempt to get logs
        self.show_container_logs(container_name);
        panic!("❌ {} failed to start within 60 seconds", container_name);
    }

    /// Show recent logs (last 20 lines) for quick debugging
    fn show_recent_logs(&self, container_name: &str) {
        let output =
            Command::new("docker").args(&["logs", "--tail", "20", container_name]).output();

        if let Ok(logs) = output {
            let stdout = String::from_utf8_lossy(&logs.stdout);
            let stderr = String::from_utf8_lossy(&logs.stderr);
            if !stdout.trim().is_empty() || !stderr.trim().is_empty() {
                println!("📋 Recent logs for {}:", container_name);
                if !stdout.trim().is_empty() {
                    println!("STDOUT:\n{}", stdout);
                }
                if !stderr.trim().is_empty() {
                    println!("STDERR:\n{}", stderr);
                }
            }
        }
    }

    /// Check if the container port is accessible
    fn check_port_accessibility(&self, container_name: &str) -> bool {
        let port = if container_name.contains("sequencer") { "8545" } else { "8547" };

        // Try to connect to the port (simple check)
        let output = Command::new("nc").args(&["-z", "localhost", port]).output();

        output.map(|o| o.status.success()).unwrap_or(false)
    }

    /// Public method to display container logs for external debugging
    pub fn show_container_logs(&self, service_name: &str) {
        println!("🔍 Getting logs for {}...", service_name);

        // Try docker-compose logs first with more lines
        let logs_output = Command::new("docker-compose")
            .args(&[
                "-f",
                &self.compose_file,
                "-p",
                &self.project_name,
                "logs",
                "--tail",
                "100",
                service_name,
            ])
            .output();

        if let Ok(logs) = logs_output {
            let stdout = String::from_utf8_lossy(&logs.stdout);
            let stderr = String::from_utf8_lossy(&logs.stderr);
            if !stdout.trim().is_empty() {
                println!("📋 {} compose logs (stdout):\n{}", service_name, stdout);
            }
            if !stderr.trim().is_empty() {
                println!("📋 {} compose logs (stderr):\n{}", service_name, stderr);
            }
        }

        // Also try direct docker logs as fallback
        let direct_logs =
            Command::new("docker").args(&["logs", "--tail", "100", service_name]).output();

        if let Ok(logs) = direct_logs {
            let stdout = String::from_utf8_lossy(&logs.stdout);
            let stderr = String::from_utf8_lossy(&logs.stderr);
            if !stdout.trim().is_empty() {
                println!("📋 {} direct logs (stdout):\n{}", service_name, stdout);
            }
            if !stderr.trim().is_empty() {
                println!("📋 {} direct logs (stderr):\n{}", service_name, stderr);
            }
        }

        // Check container internal processes
        let ps_output = Command::new("docker").args(&["exec", service_name, "ps", "aux"]).output();

        if let Ok(ps) = ps_output {
            let processes = String::from_utf8_lossy(&ps.stdout);
            println!("🔍 Processes in {}:\n{}", service_name, processes);
        } else {
            println!("⚠️ Could not get process list from {}", service_name);
        }

        // Check if rollup-node binary exists and is executable
        let binary_check =
            Command::new("docker").args(&["exec", service_name, "which", "rollup-node"]).output();

        if let Ok(output) = binary_check {
            let path = String::from_utf8_lossy(&output.stdout);
            if !path.trim().is_empty() {
                println!("🔍 rollup-node binary location: {}", path.trim());
            } else {
                println!("⚠️ rollup-node binary not found in PATH");
            }
        }
    }

    /// Show logs for all containers
    fn show_all_container_logs(compose_file: &str, project_name: &str) {
        println!("🔍 Getting all container logs...");

        let logs_output = Command::new("docker-compose")
            .args(&["-f", compose_file, "-p", project_name, "logs"])
            .output();

        if let Ok(logs) = logs_output {
            let stdout = String::from_utf8_lossy(&logs.stdout);
            let stderr = String::from_utf8_lossy(&logs.stderr);
            if !stdout.trim().is_empty() {
                eprintln!("❌ All container logs (stdout):\n{}", stdout);
            }
            if !stderr.trim().is_empty() {
                eprintln!("❌ All container logs (stderr):\n{}", stderr);
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

    /// Perform cleanup of docker-compose environment
    fn perform_cleanup(&self) {
        println!("🧹 Tearing down test environment: {}", self.project_name);

        // First try graceful docker-compose down
        let status = Command::new("docker-compose")
            .args(&[
                "-f",
                &self.compose_file,
                "-p",
                &self.project_name,
                "down",
                "--volumes",
                "--remove-orphans",
                "--timeout",
                "5",
            ])
            .status();

        match status {
            Ok(exit_status) if exit_status.success() => {
                println!("✅ Test environment cleaned up successfully");
            }
            _ => {
                eprintln!("⚠️ docker-compose down failed, forcing cleanup...");
                self.force_cleanup();
            }
        }
    }

    /// Force cleanup containers and networks manually
    fn force_cleanup(&self) {
        let containers = ["rollup-node-sequencer", "rollup-node-follower"];

        for container in &containers {
            // Get container IDs that match the name pattern
            if let Ok(output) = Command::new("docker")
                .args(&["ps", "-aq", "--filter", &format!("name={}", container)])
                .output()
            {
                let container_ids = String::from_utf8_lossy(&output.stdout);
                for id in container_ids.lines() {
                    if !id.trim().is_empty() {
                        // Force remove each container
                        let _ = Command::new("docker").args(&["rm", "-f", id.trim()]).output();
                    }
                }
            }
        }

        // Clean up project-specific network
        let network_name = format!("{}_test-scroll-network", self.project_name);
        let _ = Command::new("docker").args(&["network", "rm", &network_name]).output();

        println!("✅ Force cleanup completed");
    }
}

impl Drop for DockerComposeEnv {
    fn drop(&mut self) {
        if self.cleanup_on_drop {
            self.perform_cleanup();
        }
    }
}
