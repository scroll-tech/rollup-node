//! pprof integration for performance profiling
//!
//! This module provides HTTP endpoints for CPU profiling using pprof-rs.
//! It exposes a simple HTTP server that can be used to collect profiling data.
//!
//! # Endpoints
//! - `/debug/pprof/profile?seconds=<duration>` - CPU profile for specified duration (default: 30s)
//!
//! # Example Usage
//! ```bash
//! # Collect CPU profile for 30 seconds
//! curl http://localhost:6060/debug/pprof/profile?seconds=30 -o cpu.pb
//!
//! # Collect CPU profile for 60 seconds
//! curl http://localhost:6060/debug/pprof/profile?seconds=60 -o cpu.pb
//!
//! # View with pprof (requires Go pprof tool)
//! go tool pprof -http=:8080 cpu.pb
//!
//! # Or convert to flamegraph
//! pprof -flame cpu.pb > flamegraph.svg
//! ```

use eyre::Result;
use http_body_util::Full;
use hyper::{
    body::{Bytes, Incoming},
    server::conn::http1,
    service::service_fn,
    Method, Request, Response, StatusCode,
};
use hyper_util::rt::TokioIo;
use pprof::protos::Message;
use std::{net::SocketAddr, time::Duration};
use tokio::net::TcpListener;
use tracing::{error, info, warn};

/// Configuration for the pprof HTTP server
#[derive(Debug, Clone)]
pub struct PprofConfig {
    /// Address to bind the pprof HTTP server to
    pub addr: SocketAddr,
    /// Default profiling duration in seconds
    pub default_duration: u64,
}

impl Default for PprofConfig {
    fn default() -> Self {
        Self { addr: SocketAddr::from(([0, 0, 0, 0], 6060)), default_duration: 30 }
    }
}

impl PprofConfig {
    /// Create a new pprof configuration with custom address
    pub fn new(addr: SocketAddr) -> Self {
        Self { addr, ..Default::default() }
    }

    /// Set the default profiling duration
    pub const fn with_default_duration(mut self, seconds: u64) -> Self {
        self.default_duration = seconds;
        self
    }
}

/// Start the pprof HTTP server
///
/// This function spawns a background task that runs an HTTP server for pprof endpoints.
/// The server will run until the returned handle is dropped or the task is cancelled.
///
/// # Arguments
/// * `config` - Configuration for the pprof server
///
/// # Returns
/// A `JoinHandle` that can be used to manage the server task
///
/// # Example
/// ```no_run
/// use rollup_node::pprof::{start_pprof_server, PprofConfig};
/// use std::net::SocketAddr;
///
/// #[tokio::main]
/// async fn main() -> eyre::Result<()> {
///     let config = PprofConfig::new("127.0.0.1:6060".parse()?);
///     let handle = start_pprof_server(config).await?;
///
///     // Server runs in background
///     // ...
///
///     // Wait for server to complete (or cancel it)
///     handle.await??;
///     Ok(())
/// }
/// ```
pub async fn start_pprof_server(
    config: PprofConfig,
) -> Result<tokio::task::JoinHandle<Result<()>>> {
    let listener = TcpListener::bind(config.addr).await?;
    let addr = listener.local_addr()?;

    info!("Starting pprof server on http://{}", addr);
    info!("CPU profile endpoint: http://{}/debug/pprof/profile?seconds=30", addr);

    let handle = tokio::spawn(async move {
        loop {
            let (stream, peer_addr) = match listener.accept().await {
                Ok(conn) => conn,
                Err(e) => {
                    error!("Failed to accept connection: {}", e);
                    continue;
                }
            };

            let io = TokioIo::new(stream);
            let default_duration = config.default_duration;

            tokio::spawn(async move {
                let service = service_fn(move |req| handle_request(req, default_duration));

                if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                    error!("Error serving connection from {}: {}", peer_addr, err);
                }
            });
        }
    });

    Ok(handle)
}

/// Handle HTTP requests to pprof endpoints
async fn handle_request(
    req: Request<Incoming>,
    default_duration: u64,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/debug/pprof/profile") => {
            // Parse duration from query parameters
            let duration = req
                .uri()
                .query()
                .and_then(|q| {
                    q.split('&')
                        .find(|pair| pair.starts_with("seconds="))
                        .and_then(|pair| pair.strip_prefix("seconds="))
                        .and_then(|s| s.parse::<u64>().ok())
                })
                .unwrap_or(default_duration);

            info!("Starting CPU profile for {} seconds", duration);
            handle_cpu_profile(duration).await
        }
        (&Method::GET, "/" | "/debug/pprof") => handle_index().await,
        _ => {
            warn!("Not found: {} {}", req.method(), req.uri().path());
            Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Full::new(Bytes::from("Not Found")))
                .unwrap())
        }
    }
}

/// Handle CPU profiling requests
async fn handle_cpu_profile(duration_secs: u64) -> Result<Response<Full<Bytes>>, hyper::Error> {
    // Validate duration
    if duration_secs == 0 || duration_secs > 600 {
        let error_msg = "Invalid duration: must be between 1 and 600 seconds";
        warn!("{}", error_msg);
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Full::new(Bytes::from(error_msg)))
            .unwrap());
    }

    info!("Collecting CPU profile for {} seconds...", duration_secs);

    // Start profiling
    let guard = match pprof::ProfilerGuardBuilder::default().build() {
        Ok(guard) => guard,
        Err(e) => {
            error!("Failed to start profiler: {}", e);
            let error_msg = format!("Failed to start profiler: {}", e);
            return Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Full::new(Bytes::from(error_msg)))
                .unwrap());
        }
    };

    // Profile for the specified duration
    tokio::time::sleep(Duration::from_secs(duration_secs)).await;

    // Generate report
    match guard.report().build() {
        Ok(report) => {
            // Encode as protobuf
            match report.pprof() {
                Ok(profile) => {
                    // The profile object needs to be converted to bytes
                    let body = match profile.write_to_bytes() {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            error!("Failed to encode profile: {}", e);
                            let error_msg = format!("Failed to encode profile: {}", e);
                            return Ok(Response::builder()
                                .status(StatusCode::INTERNAL_SERVER_ERROR)
                                .body(Full::new(Bytes::from(error_msg)))
                                .unwrap());
                        }
                    };

                    info!("Successfully collected CPU profile ({} bytes)", body.len());

                    Ok(Response::builder()
                        .status(StatusCode::OK)
                        .header("Content-Type", "application/octet-stream")
                        .header(
                            "Content-Disposition",
                            format!("attachment; filename=\"profile-{}.pb\"", duration_secs),
                        )
                        .body(Full::new(Bytes::from(body)))
                        .unwrap())
                }
                Err(e) => {
                    error!("Failed to generate pprof format: {}", e);
                    let error_msg = format!("Failed to generate pprof format: {}", e);
                    Ok(Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Full::new(Bytes::from(error_msg)))
                        .unwrap())
                }
            }
        }
        Err(e) => {
            error!("Failed to generate report: {}", e);
            let error_msg = format!("Failed to generate report: {}", e);
            Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Full::new(Bytes::from(error_msg)))
                .unwrap())
        }
    }
}

/// Handle index/help page requests
async fn handle_index() -> Result<Response<Full<Bytes>>, hyper::Error> {
    let body = r#"<!DOCTYPE html>
<html>
<head>
    <title>pprof</title>
</head>
<body>
<h1>pprof - Performance Profiling</h1>
<p>Available endpoints:</p>
<ul>
    <li><a href="/debug/pprof/profile?seconds=30">/debug/pprof/profile?seconds=30</a> - CPU profile (30 seconds)</li>
    <li><a href="/debug/pprof/profile?seconds=60">/debug/pprof/profile?seconds=60</a> - CPU profile (60 seconds)</li>
</ul>

<h2>Usage Examples</h2>
<pre>
# Collect CPU profile
curl http://localhost:6060/debug/pprof/profile?seconds=30 -o cpu.pb

# Analyze with Go pprof
go tool pprof -http=:8080 cpu.pb

# Generate flamegraph
pprof -flame cpu.pb > flamegraph.svg
</pre>

<h2>Documentation</h2>
<p>
This service uses <a href="https://github.com/tikv/pprof-rs">pprof-rs</a> for performance profiling.
The output is compatible with Google's pprof format and can be analyzed using various tools.
</p>
</body>
</html>
"#;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/html; charset=utf-8")
        .body(Full::new(Bytes::from(body)))
        .unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = PprofConfig::default();
        assert_eq!(config.addr, "0.0.0.0:6868".parse::<SocketAddr>().unwrap());
        assert_eq!(config.default_duration, 30);
    }

    #[test]
    fn test_custom_config() {
        let addr = "0.0.0.0:6868".parse::<SocketAddr>().unwrap();
        let config = PprofConfig::new(addr).with_default_duration(60);
        assert_eq!(config.addr, addr);
        assert_eq!(config.default_duration, 60);
    }
}
