/// The sync state of the chain orchestrator.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SyncState {
    /// The sync mode for L1.
    l1: SyncMode,
    /// The sync mode for L2.
    l2: SyncMode,
}

impl Default for SyncState {
    fn default() -> Self {
        Self { l1: SyncMode::default(), l2: SyncMode::Synced }
    }
}

impl SyncState {
    /// Returns a reference to the sync mode of L1.
    pub const fn l1(&self) -> &SyncMode {
        &self.l1
    }

    /// Returns a reference to the sync mode of L2.
    pub const fn l2(&self) -> &SyncMode {
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
    pub const fn is_synced(&self) -> bool {
        self.l1.is_synced() && self.l2.is_synced()
    }
}

/// The sync mode of the chain orchestrator.
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SyncMode {
    /// Syncing mode.
    #[default]
    Syncing,
    /// Synced mode.
    Synced,
}

impl SyncMode {
    /// Returns true if the sync mode is [`SyncMode::Syncing`].
    pub const fn is_syncing(&self) -> bool {
        matches!(self, Self::Syncing)
    }

    /// Returns true if the sync mode is [`SyncMode::Synced`].
    pub const fn is_synced(&self) -> bool {
        matches!(self, Self::Synced)
    }

    /// Sets the sync mode to [`SyncMode::Synced`].
    pub fn set_synced(&mut self) {
        *self = Self::Synced;
    }

    /// Sets the sync mode to [`SyncMode::Syncing`].
    pub fn set_syncing(&mut self) {
        *self = Self::Syncing;
    }
}
