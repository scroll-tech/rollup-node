//! Command handle for the L1 Watcher.

mod command;

pub use command::L1WatcherCommand;

use crate::L1Notification;
use std::sync::Arc;
use tokio::sync::{mpsc, mpsc::UnboundedSender, oneshot};

/// Trait for interacting with the L1 Watcher.
///
/// This trait allows the chain orchestrator to send commands to the L1 watcher,
/// primarily for gap recovery scenarios.
#[async_trait::async_trait]
pub trait L1WatcherHandleTrait: Send + Sync + 'static {
    /// Reset the L1 Watcher to a specific block number with a fresh notification channel.
    ///
    /// This is used for gap recovery when the chain orchestrator detects missing L1 events.
    /// The watcher will reset its state to the specified block and begin sending notifications
    /// through the new channel.
    ///
    /// # Arguments
    /// * `block` - The L1 block number to reset to
    /// * `new_sender` - A fresh channel sender for L1 notifications
    ///
    /// # Returns
    /// `Ok(())` if the reset was successful, or an error if the command failed
    async fn reset_to_block(
        &self,
        block: u64,
        new_sender: mpsc::Sender<Arc<L1Notification>>,
    ) -> Result<(), oneshot::error::RecvError>;
}

/// Handle to interact with the L1 Watcher.
#[derive(Debug)]
pub struct L1WatcherHandle {
    to_watcher_tx: UnboundedSender<L1WatcherCommand>,
}

impl L1WatcherHandle {
    /// Create a new handle with the given command sender.
    pub const fn new(to_watcher_tx: UnboundedSender<L1WatcherCommand>) -> Self {
        Self { to_watcher_tx }
    }

    /// Send a command to the watcher without waiting for a response.
    fn send_command(&self, command: L1WatcherCommand) {
        if let Err(err) = self.to_watcher_tx.send(command) {
            tracing::error!(target: "scroll::watcher", ?err, "Failed to send command to L1 watcher");
        }
    }

    /// Reset the L1 Watcher to a specific block number with a fresh notification channel.
    ///
    /// Returns an error if the command could not be delivered or the watcher
    /// dropped the response channel.
    pub async fn reset_to_block(
        &self,
        block: u64,
        new_sender: mpsc::Sender<Arc<L1Notification>>,
    ) -> Result<(), oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        self.send_command(L1WatcherCommand::ResetToBlock {
            block,
            new_sender,
            response_sender: tx,
        });
        rx.await
    }
}

#[async_trait::async_trait]
impl L1WatcherHandleTrait for L1WatcherHandle {
    async fn reset_to_block(
        &self,
        block: u64,
        new_sender: mpsc::Sender<Arc<L1Notification>>,
    ) -> Result<(), oneshot::error::RecvError> {
        self.reset_to_block(block, new_sender).await
    }
}

#[cfg(any(test, feature = "test-utils"))]
/// Mock implementation of L1WatcherHandleTrait for testing.
///
/// This mock tracks all reset calls for test assertions and always succeeds.
#[derive(Debug, Clone)]
pub struct MockL1WatcherHandle {
    /// Track reset calls as (block_number, channel_capacity)
    resets: Arc<std::sync::Mutex<Vec<(u64, usize)>>>,
}

#[cfg(any(test, feature = "test-utils"))]
impl MockL1WatcherHandle {
    /// Create a new mock handle.
    pub fn new() -> Self {
        Self {
            resets: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    /// Get all recorded reset calls as (block_number, channel_capacity).
    pub fn get_resets(&self) -> Vec<(u64, usize)> {
        self.resets.lock().unwrap().clone()
    }

    /// Assert that reset_to_block was called with the specified block number.
    pub fn assert_reset_to(&self, expected_block: u64) {
        let resets = self.get_resets();
        assert!(
            resets.iter().any(|(block, _)| *block == expected_block),
            "Expected reset to block {}, but got resets: {:?}",
            expected_block,
            resets
        );
    }

    /// Assert that no reset calls were made.
    pub fn assert_no_resets(&self) {
        let resets = self.get_resets();
        assert!(
            resets.is_empty(),
            "Expected no reset calls, but got: {:?}",
            resets
        );
    }
}
#[cfg(any(test, feature = "test-utils"))]
#[async_trait::async_trait]
impl L1WatcherHandleTrait for MockL1WatcherHandle {
    async fn reset_to_block(
        &self,
        block: u64,
        new_sender: mpsc::Sender<Arc<L1Notification>>,
    ) -> Result<(), oneshot::error::RecvError> {
        // Track the reset call
        self.resets
            .lock()
            .unwrap()
            .push((block, new_sender.max_capacity()));

        // Mock always succeeds
        Ok(())
    }
}
