//! Scroll binary

#[global_allocator]
static ALLOC: reth_cli_util::allocator::Allocator = reth_cli_util::allocator::new_allocator();

fn main() {
    use clap::Parser;
    use reth_node_builder::EngineNodeLauncher;
    use reth_scroll_cli::{Cli, ScrollChainSpecParser};
    use rollup_node::{ScrollRollupNode, ScrollRollupNodeConfig};
    use tracing::info;

    // Initialize tracing subscriber with optional tokio-console support
    init_tracing_subscriber();

    reth_cli_util::sigsegv_handler::install();

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }

    if let Err(err) = Cli::<ScrollChainSpecParser, ScrollRollupNodeConfig>::parse().run(
        |builder, args| async move {
            info!(target: "reth::cli", "Launching node");
            let config = builder.config().clone();
            let handle = builder
                .node(ScrollRollupNode::new(args, config).await)
                .launch_with_fn(|builder| {
                    info!(target: "reth::cli", config = ?builder.config().chain.config, "Running with config");

                    // We must use `always_process_payload_attributes_on_canonical_head` in order to
                    // be able to build payloads with the forkchoice state API
                    // on top of heads part of the canonical state. Not
                    // providing this argument leads the `EngineTree` to ignore
                    // the payload building attributes: <https://github.com/scroll-tech/reth/blob/4271872fdcbe7ff96520825e38f5e36ef923fcca/crates/engine/tree/src/tree/mod.rs#L898>
                    let tree_config = builder
                        .config()
                        .engine
                        .tree_config()
                        .with_always_process_payload_attributes_on_canonical_head(true).with_persistence_threshold(0).with_unwind_canonical_header(true);
                    let launcher = EngineNodeLauncher::new(
                        builder.task_executor().clone(),
                        builder.config().datadir(),
                        tree_config,
                    );
                    builder.launch_with(launcher)
                })
                .await?;
            handle.node_exit_future.await
        },
    ) {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}

/// Initialize tracing subscriber with optional tokio-console support
///
/// This function sets up a layered tracing subscriber that:
/// - Always outputs logs to the console (preserving original behavior)
/// - Optionally enables tokio-console when `TOKIO_CONSOLE=1` environment variable is set
///
/// To use tokio-console, set the environment variable and connect with:
/// ```bash
/// TOKIO_CONSOLE=1 cargo run ...
/// tokio-console  # in another terminal
/// ```
fn init_tracing_subscriber() {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    // Check if tokio-console should be enabled
    let enable_tokio_console = std::env::var("TOKIO_CONSOLE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    // Create optional console layer
    #[allow(clippy::if_then_some_else_none)]
    let console_layer = if enable_tokio_console {
        eprintln!("✓ Tokio-console enabled. Connect with: tokio-console");
        Some(console_subscriber::ConsoleLayer::builder().with_default_env().spawn())
    } else {
        None
    };

    // Initialize subscriber with layers
    tracing_subscriber::registry()
        .with(console_layer)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_thread_ids(false)
                .with_line_number(false)
                .with_ansi(true),
        )
        .with(
            EnvFilter::try_from_default_env()
                .or_else(|_| EnvFilter::try_new("info"))
                .expect("Failed to create EnvFilter"),
        )
        .init();
}
