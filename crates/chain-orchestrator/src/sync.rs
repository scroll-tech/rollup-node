/// The sync state of the chain orchestrator.
#[derive(Debug, Default, Clone)]
pub struct SyncState {
    /// The sync mode for L1.
    l1: SyncMode,
    /// The sync mode for L2.
    l2: SyncMode,
}

impl SyncState {
    /// Returns a reference to the sync mode of L1.
    pub fn l1(&self) -> &SyncMode {
        &self.l1
    }

    /// Returns a reference to the sync mode of L2.
    pub fn l2(&self) -> &SyncMode {
        &self.l2
    }

    /// Returns a mutable reference to the sync mode of L1.
    pub fn l1_mut(&mut self) -> &mut SyncMode {
        &mut self.l1
    }

    /// Returns a mutable reference to the sync mode of L2.
    pub fn l2_mut(&mut self) -> &mut SyncMode {
        &mut self.l2
    }

    /// Returns true if both L1 and L2 are synced.
    pub fn is_synced(&self) -> bool {
        self.l1.is_synced() && self.l2.is_synced()
    }
}

/// The sync mode of the chain orchestrator.
#[derive(Debug, Default, Clone)]
pub enum SyncMode {
    /// Syncing mode.
    #[default]
    Syncing,
    /// Synced mode.
    Synced,
}

impl SyncMode {
    /// Returns true if the sync mode is [`Self::Syncing`].
    pub fn is_syncing(&self) -> bool {
        matches!(self, Self::Syncing)
    }

    pub fn is_synced(&self) -> bool {
        matches!(self, Self::Synced)
    }

    /// Sets the sync mode to [`Self::Synced].
    pub fn set_synced(&mut self) {
        *self = Self::Synced;
    }

    /// Sets the sync mode to [`Self::Syncing`].
    pub fn set_syncing(&mut self) {
        *self = Self::Syncing;
    }
}
