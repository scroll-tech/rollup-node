//! Database operation helpers for test fixtures.

use super::fixture::TestFixture;
use scroll_db::{Database, DatabaseError, DatabaseReadOperations};
use std::sync::Arc;

/// Helper for performing database operations on a test node.
#[derive(Debug)]
pub struct DatabaseHelper<'a> {
    fixture: &'a mut TestFixture,
    node_index: usize,
}

impl<'a> DatabaseHelper<'a> {
    /// Create a new database helper for a specific node.
    pub const fn new(fixture: &'a mut TestFixture, node_index: usize) -> Self {
        Self { fixture, node_index }
    }

    /// Get the database for this node.
    async fn database(&self) -> eyre::Result<Arc<Database>> {
        let node = self.fixture.nodes[self.node_index]
            .as_ref()
            .ok_or_else(|| eyre::eyre!("Node at index {} has been shutdown", self.node_index))?;
        Ok(node.rollup_manager_handle.get_database_handle().await?)
    }

    /// Get the finalized block number of a batch by its index.
    ///
    /// Returns `Ok(None)` if the batch is not found, or `Ok(Some(None))` if the batch exists
    /// but has no finalized block number yet.
    pub async fn get_batch_finalized_block_number_by_index(
        &self,
        index: u64,
    ) -> eyre::Result<Option<Option<u64>>> {
        let db = self.database().await?;
        let batch = db
            .tx_mut(move |tx| async move {
                let batch = tx.get_batch_by_index(index).await?;
                Ok::<_, DatabaseError>(batch)
            })
            .await?;

        Ok(batch.map(|b| b.finalized_block_number))
    }
}

/// Extension trait for `TestFixture` to add database operation capabilities.
pub trait DatabaseOperations {
    /// Get database helper for the sequencer node.
    fn db(&mut self) -> DatabaseHelper<'_>;

    /// Get database helper for a specific node.
    fn db_on(&mut self, node_index: usize) -> DatabaseHelper<'_>;
}

impl DatabaseOperations for TestFixture {
    fn db(&mut self) -> DatabaseHelper<'_> {
        DatabaseHelper::new(self, 0)
    }

    fn db_on(&mut self, node_index: usize) -> DatabaseHelper<'_> {
        DatabaseHelper::new(self, node_index)
    }
}
