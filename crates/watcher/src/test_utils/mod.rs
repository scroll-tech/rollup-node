use crate::{random, Header, L1Notification, L1WatcherCommand};
use arbitrary::Arbitrary;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Test utils for arbitrary.
pub mod arbitrary;

/// Test utils for provider.
pub mod provider;

/// Mock for the L1 Watcher.
///
/// This allows tests to simulate L1 watcher behavior by sending commands and receiving
/// notifications.
#[derive(Clone, Debug)]
pub struct L1WatcherMock {
    /// Receiver for L1 watcher commands.
    pub command_rx: Option<Arc<Mutex<mpsc::UnboundedReceiver<L1WatcherCommand>>>>,
    /// Sender for L1 notifications.
    pub notification_tx: mpsc::Sender<Arc<L1Notification>>,
}

impl L1WatcherMock {
    /// Handle commands sent to the L1 watcher mock.
    pub async fn handle_command(&mut self) {
        if let Some(command_rx) = &self.command_rx {
            let mut commands = command_rx.lock().await;
            if let Some(command) = commands.recv().await {
                match command {
                    L1WatcherCommand::ResetToBlock { block, tx } => {
                        // For testing purposes, we can just log the reset action.
                        tracing::info!(target: "scroll::watcher::test_utils", "L1 Watcher Mock resetting to block {}", block);
                        self.notification_tx = tx;
                    }
                }
            }
        }
    }
}

/// Returns a chain of random headers of size `len`.
pub fn chain(len: usize) -> (Header, Header, Vec<Header>) {
    assert!(len >= 2, "chain should have a minimal length of two");

    let mut chain = Vec::with_capacity(len);
    chain.push(random!(Header));
    for i in 1..len {
        let mut next = random!(Header);
        next.number = chain[i - 1].number + 1;
        next.parent_hash = chain[i - 1].hash;
        chain.push(next);
    }

    (chain.first().unwrap().clone(), chain.last().unwrap().clone(), chain)
}

/// Returns a chain of random block of size `len`, starting at the provided header.
pub fn chain_from(header: &Header, len: usize) -> Vec<Header> {
    assert!(len >= 2, "fork should have a minimal length of two");

    let mut blocks = Vec::with_capacity(len);
    blocks.push(header.clone());

    let next_header = |header: &Header| {
        let mut next = random!(Header);
        next.parent_hash = header.hash;
        next.number = header.number + 1;
        next
    };
    for i in 0..len - 1 {
        blocks.push(next_header(&blocks[i]));
    }
    blocks
}
