use scroll_engine::ForkchoiceState;

pub struct State {
    fcs: ForkchoiceState,
    is_syncing: bool,
}
