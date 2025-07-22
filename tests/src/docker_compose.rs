use std::{
    process::Command,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub struct DockerComposeEnv {
    project_name: String,
    compose_file: String,
    cleanup_on_drop: bool,
}

impl DockerComposeEnv {
    pub fn new(test_name: &str) -> Self {
        let start = SystemTime::now();
        let since_the_epoch = start.duration_since(UNIX_EPOCH).expect("Time went backwards");
        let unique_id = since_the_epoch.as_nanos();
        let project_name = format!("test-{test_name}-{unique_id}");
        let compose_file = "docker-compose.test.yml".to_string();

        println!("🚀 Starting test environment: {project_name}");

        // Pre-cleanup existing containers to avoid conflicts
        Self::cleanup_existing_containers();

        // Start the environment
        Self::start_environment(&compose_file, &project_name)
    }

    /// Clean up any existing containers with the same names to avoid conflicts
    fn cleanup_existing_containers() {
        println!("🧹 Pre-cleaning existing containers...");

        let containers = ["rollup-node-sequencer", "rollup-node-follower"];
        for container in &containers {
            // Stop container if running
            let _ = Command::new("docker").args(["stop", container]).output();

            // Remove container forcefully
            let _ = Command::new("docker").args(["rm", "-f", container]).output();
        }

        // Clean up orphaned networks
        let _ = Command::new("docker").args(["network", "prune", "-f"]).output();
    }

    fn start_environment(compose_file: &str, project_name: &str) -> Self {
        println!("📦 Starting docker-compose services...");

        let mut child = Command::new("docker-compose")
            .args([
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
                .args(["inspect", "--format={{.State.Running}}", container_name])
                .output();

            if let Ok(output) = output {
                let is_running = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if is_running == "true" {
                    println!("✅ {container_name} is running");

                    // Additional check: try to connect to the port
                    if self.check_port_accessibility(container_name) {
                        return;
                    }
                } else if is_running == "false" {
                    // Container stopped, get exit code and logs
                    let exit_code_output = Command::new("docker")
                        .args(["inspect", "--format={{.State.ExitCode}}", container_name])
                        .output();

                    if let Ok(exit_output) = exit_code_output {
                        let exit_code_str = String::from_utf8_lossy(&exit_output.stdout);
                        let exit_code = exit_code_str.trim();
                        eprintln!("❌ {container_name} exited with code: {exit_code}");
                    }

                    self.show_container_logs(container_name);
                    panic!("❌ {container_name} failed to start (container exited)");
                }
            }

            if attempt % 5 == 0 {
                println!("⏳ Waiting for {container_name}... (attempt {attempt}/30)");
                // Show recent logs every 5 attempts
                self.show_recent_logs(container_name);
            }

            thread::sleep(Duration::from_secs(2));
        }

        // Final attempt to get logs
        self.show_container_logs(container_name);
        panic!("❌ {container_name} failed to start within 60 seconds");
    }

    /// Show recent logs (last 20 lines) for quick debugging
    fn show_recent_logs(&self, container_name: &str) {
        let output = Command::new("docker").args(["logs", "--tail", "20", container_name]).output();

        if let Ok(logs) = output {
            let stdout = String::from_utf8_lossy(&logs.stdout);
            let stderr = String::from_utf8_lossy(&logs.stderr);
            if !stdout.trim().is_empty() || !stderr.trim().is_empty() {
                println!("📋 Recent logs for {container_name}:");
                if !stdout.trim().is_empty() {
                    println!("STDOUT:\n{stdout}");
                }
                if !stderr.trim().is_empty() {
                    println!("STDERR:\n{stderr}");
                }
            }
        }
    }

    /// Check if the container port is accessible
    fn check_port_accessibility(&self, container_name: &str) -> bool {
        let port = if container_name.contains("sequencer") { "8545" } else { "8547" };

        // Try to connect to the port (simple check)
        let output = Command::new("nc").args(["-z", "localhost", port]).output();

        output.map(|o| o.status.success()).unwrap_or(false)
    }

    /// Public method to display container logs for external debugging
    pub fn show_container_logs(&self, service_name: &str) {
        println!("🔍 Getting logs for {service_name}...");

        // Try docker-compose logs first with more lines
        let logs_output = Command::new("docker-compose")
            .args([
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
                println!("📋 {service_name} compose logs (stdout):\n{stdout}");
            }
            if !stderr.trim().is_empty() {
                println!("📋 {service_name} compose logs (stderr):\n{stderr}");
            }
        }

        // Also try direct docker logs as fallback
        let direct_logs =
            Command::new("docker").args(["logs", "--tail", "100", service_name]).output();

        if let Ok(logs) = direct_logs {
            let stdout = String::from_utf8_lossy(&logs.stdout);
            let stderr = String::from_utf8_lossy(&logs.stderr);
            if !stdout.trim().is_empty() {
                println!("📋 {service_name} direct logs (stdout):\n{stdout}");
            }
            if !stderr.trim().is_empty() {
                println!("📋 {service_name} direct logs (stderr):\n{stderr}");
            }
        }
    }

    /// Show logs for all containers
    fn show_all_container_logs(compose_file: &str, project_name: &str) {
        println!("🔍 Getting all container logs...");

        let logs_output = Command::new("docker-compose")
            .args(["-f", compose_file, "-p", project_name, "logs"])
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

    /// Perform cleanup of docker-compose environment
    fn perform_cleanup(&self) {
        println!("🧹 Tearing down test environment: {0}", self.project_name);

        // First try graceful docker-compose down
        let status = Command::new("docker-compose")
            .args([
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
        // Clean up project-specific network
        let network_name = format!("{}_test-scroll-network", self.project_name);
        let _ = Command::new("docker").args(["network", "rm", &network_name]).output();

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
