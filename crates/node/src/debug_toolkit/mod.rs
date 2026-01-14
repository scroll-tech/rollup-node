//! Debug Toolkit for Scroll Rollup Node
//!
//! This module provides an interactive REPL and debugging utilities for
//! hackathons, development, and debugging scenarios.
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use rollup_node::debug_toolkit::prelude::*;
//! use rollup_node::test_utils::TestFixture;
//!
//! #[tokio::main]
//! async fn main() -> eyre::Result<()> {
//!     // Create a test fixture
//!     let fixture = TestFixture::builder()
//!         .with_chain("dev")
//!         .sequencer()
//!         .with_noop_consensus()
//!         .build()
//!         .await?;
//!
//!     // Start the REPL
//!     let mut repl = DebugRepl::new(fixture);
//!     repl.run().await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! # Custom Actions
//!
//! You can create custom actions by implementing the [`actions::Action`] trait:
//!
//! ```rust,ignore
//! use rollup_node::debug_toolkit::actions::{Action, ActionRegistry};
//! use rollup_node::test_utils::TestFixture;
//! use async_trait::async_trait;
//!
//! struct MyAction;
//!
//! #[async_trait]
//! impl Action for MyAction {
//!     fn name(&self) -> &'static str { "my-action" }
//!     fn description(&self) -> &'static str { "Does something cool" }
//!
//!     async fn execute(
//!         &self,
//!         fixture: &mut TestFixture,
//!         args: &[String],
//!     ) -> eyre::Result<()> {
//!         // Your logic here with full fixture access
//!         Ok(())
//!     }
//! }
//! ```

pub mod actions;
pub mod cli;
mod commands;
mod event_stream;
mod repl;

pub use cli::DebugArgs;
pub use commands::*;
pub use event_stream::*;
pub use repl::*;

/// Prelude for convenient imports.
pub mod prelude {
    pub use super::{
        actions::{Action, ActionRegistry},
        DebugRepl, EventStreamState,
    };
}
