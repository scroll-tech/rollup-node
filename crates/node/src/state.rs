use scroll_engine::ForkchoiceState;

/// The state of the node.
#[derive(Debug)]
pub struct State {
    /// The forkchoice state of the node.
    fcs: ForkchoiceState,
    /// Whether the node is syncing.
    is_syncing: bool,
}

impl State {
    /// Create a new [`State`] instance.
    pub const fn new(fcs: ForkchoiceState, is_syncing: bool) -> Self {
        Self { fcs, is_syncing }
    }

    /// Get a reference to the [`ForkchoiceState`] of the node.
    pub const fn fcs(&self) -> &ForkchoiceState {
        &self.fcs
    }

    /// Returns a bool indicating whether the node is syncing.
    pub const fn is_syncing(&self) -> bool {
        self.is_syncing
    }
}
