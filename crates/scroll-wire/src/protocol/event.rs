use alloy_primitives::PrimitiveSignature;

/// The events that can be emitted by the ScrollWire protocol.
#[derive(Debug)]
pub enum ScrollWireEvent {
    NewBlock {
        // TODO: This should include a peer_id such that we can track which peer sent the block and
        // assign reputation accordingly based on the block's validity.
        block: reth_primitives::Block,
        signature: PrimitiveSignature,
    },
}
