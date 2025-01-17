use alloy_primitives::PrimitiveSignature;
use reth_network_peers::PeerId;
use scroll_wire::NewBlock;
use std::task::{Context, Poll};

/// A trait for importing new blocks from the network.
pub trait BlockImport: std::fmt::Debug + Send + Sync {
    /// Called when a new block is received from the network.
    fn on_new_block(
        &mut self,
        peer_id: PeerId,
        block: reth_primitives::Block,
        signature: PrimitiveSignature,
    );

    /// Polls the block import type for results of block import.
    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<BlockImportOutcome>;
}

/// The outcome of a block import operation.
pub struct BlockImportOutcome {
    /// The peer that the block was received from.
    pub peer: PeerId,
    /// The result of the block import operation.
    pub result: Result<BlockValidation, BlockImportError>,
}

/// The result of a block validation operation.
pub enum BlockValidation {
    /// The block header is valid.
    ValidHeader { new_block: NewBlock },
    /// The block is valid.
    ValidBlock { new_block: NewBlock },
}

/// An error that can occur during block import.
pub enum BlockImportError {
    /// An error occurred during consensus.
    Consensus(ConsensusError),
}

/// A consensus related error that can occur during block import.
pub enum ConsensusError {
    /// The block is invalid.
    InvalidBlock,
    /// The state root is invalid.
    InvalidStateRoot,
    /// The signature is invalid.
    InvalidSignature,
}

/// A block import type that does nothing.
#[derive(Debug)]
pub struct NoopBlockImport;

impl BlockImport for NoopBlockImport {
    fn on_new_block(
        &mut self,
        _peer_id: PeerId,
        _block: reth_primitives::Block,
        _signature: PrimitiveSignature,
    ) {
        println!("received new block");
    }

    fn poll(&mut self, _cx: &mut std::task::Context<'_>) -> std::task::Poll<BlockImportOutcome> {
        std::task::Poll::Pending
    }
}
