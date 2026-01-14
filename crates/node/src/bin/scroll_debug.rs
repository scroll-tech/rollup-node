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

    // Initialize tracing
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }
    tracing_subscriber::fmt::init();

    // Create tokio runtime and run
    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(async {
        let args = DebugArgs::parse();
        args.run().await
    })
}

#[cfg(not(feature = "debug-toolkit"))]
fn main() {
    eprintln!("Error: scroll-debug requires the 'debug-toolkit' feature.");
    eprintln!("Run with: cargo run --features debug-toolkit --bin scroll-debug");
    std::process::exit(1);
}
