//! Pipeline for processing batch inputs.

use futures::Stream;
use scroll_alloy_rpc_types_engine::ScrollPayloadAttributes;
use scroll_primitives::BatchInput;

/// A pipeline for processing batch inputs and producing scroll payloads.
#[derive(Debug)]
pub struct Pipeline;

impl Pipeline {
    /// Creates a new [`Pipeline`] instance.
    pub fn new() -> Self {
        Pipeline
    }

    /// Handles a batch input.
    pub fn handle_batch_input(&mut self, _batch_input: BatchInput) {
        // Handle the batch input.
        todo!()
    }

    /// Gets the next scroll payload.
    pub fn next(&mut self) -> Option<ScrollPayloadAttributes> {
        // Get the next scroll payload.
        todo!()
    }
}

impl Stream for Pipeline {
    type Item = ScrollPayloadAttributes;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();

        todo!()
    }
}
