//! Scroll Debug Toolkit - Interactive REPL for debugging rollup nodes.
//!
//! Usage:
//! ```bash
//! # Start REPL with dev chain and sequencer mode
//! cargo run --features debug-toolkit --bin scroll-debug -- --chain dev --sequencer
//!
//! # Start with persistent storage
//! cargo run --features debug-toolkit --bin scroll-debug -- --chain dev --sequencer --datadir ./data
//!
//! # Start with followers
//! cargo run --features debug-toolkit --bin scroll-debug -- --chain dev --sequencer --followers 2
//!
//! # Start with a real L1 endpoint
//! cargo run --features debug-toolkit --bin scroll-debug -- --chain dev --sequencer --l1-url https://eth.llamarpc.com
//!
//! # See all available options
//! cargo run --features debug-toolkit --bin scroll-debug -- --help
//! ```

#[cfg(feature = "debug-toolkit")]
fn main() -> eyre::Result<()> {
    use clap::Parser;
    use rollup_node::debug_toolkit::DebugArgs;
    use std::fs::File;
    use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    // Parse args first so we can use log_file option
    let args = DebugArgs::parse();

    // Set default log level
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }

    // Determine log file path (default to current directory)
    let log_path = args.log_file.clone().unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(format!("scroll-debug-{}.log", std::process::id()))
    });

    // Initialize tracing to write to file
    let file = File::create(&log_path)?;
    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(fmt::layer().with_writer(file).with_ansi(false))
        .init();

    eprintln!("Logs: {}", log_path.display());
    eprintln!("Tail: tail -f {}", log_path.display());
    eprintln!();
    eprintln!("Starting nodes (this may take a moment)...");

    // Create tokio runtime and run
    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(async {
        args.run(Some(log_path)).await
    })
}

#[cfg(not(feature = "debug-toolkit"))]
fn main() {
    eprintln!("Error: scroll-debug requires the 'debug-toolkit' feature.");
    eprintln!("Run with: cargo run --features debug-toolkit --bin scroll-debug");
    std::process::exit(1);
}
