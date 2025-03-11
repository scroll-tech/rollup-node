use scroll_primitives::BatchInput;

/// A provider for batch data.
pub trait BatchInputProvider {
    fn get_batch_input(&self, batch_index: u64) -> Option<BatchInput>;
    fn insert_batch_input(&self, batch: BatchInput);
}
