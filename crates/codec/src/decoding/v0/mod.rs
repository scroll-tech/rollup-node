//! Implements the V1 decoding of the calldata into a list of L2 blocks.

pub use batch_header::BatchHeaderV0;
mod batch_header;

pub(crate) use block_context::BlockContextV0;
mod block_context;

use crate::{
    decoding::{batch::Batch, payload::PayloadData, transaction::Transaction},
    error::DecodingError,
    L2Block,
};
use std::vec::Vec;

use alloy_primitives::bytes::Buf;
use scroll_l1::abi::calls::CommitBatchCall;

/// Decodes the input calldata into a [`Vec<L2Block>`].
pub fn decode_v0(calldata: &[u8]) -> Result<Batch, DecodingError> {
    // abi decode into a commit batch call
    let call = CommitBatchCall::try_decode(calldata).ok_or(DecodingError::InvalidCalldataFormat)?;

    let mut chunks_block_count = Vec::new();
    let mut l2_blocks: Vec<L2Block> = Vec::new();

    // iterate the chunks
    for chunk in call.chunks().ok_or(DecodingError::MissingChunkData)? {
        let buf = &mut &*chunk;

        // get the block count
        let blocks_count = buf.first().copied().ok_or(DecodingError::Eof)? as usize;
        chunks_block_count.push(blocks_count);
        buf.advance(1);

        let mut block_contexts: Vec<BlockContextV0> = Vec::with_capacity(blocks_count);
        l2_blocks.reserve(blocks_count);

        // for each block, decode into a block context
        for _ in 0..blocks_count {
            let context = BlockContextV0::try_from_buf(buf)?;
            block_contexts.push(context);
        }

        // for each block context, decode the transactions
        for context in block_contexts {
            let transactions_count = context.transactions_count();
            let mut transactions = Vec::with_capacity(transactions_count);
            for _ in 0..transactions_count {
                // skip the 4 bytes representing the transaction length.
                buf.advance(4);
                let tx = Transaction::try_from_buf(buf).ok_or(DecodingError::Eof)?;
                transactions.push(tx.0);
            }
            l2_blocks.push(L2Block::new(transactions, context.into()))
        }
    }

    // decode the parent batch header.
    let raw_parent_header = call.parent_batch_header().ok_or(DecodingError::MissingParentHeader)?;
    let parent_header = BatchHeaderV0::try_from_buf(&mut (&*raw_parent_header))?;
    let l1_message_start_index = parent_header.total_l1_message_popped;

    let payload = PayloadData {
        blocks: l2_blocks,
        l1_message_queue_info: l1_message_start_index.into(),
        skipped_l1_message_bitmap: call.skipped_l1_message_bitmap(),
    };

    Ok(Batch::new(call.version(), Some(chunks_block_count), payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decoding::test_utils::read_to_bytes, BlockContext};

    use alloy_primitives::{bytes, U256};

    #[test]
    fn test_should_decode_v0() -> eyre::Result<()> {
        // <https://etherscan.io/tx/0x2c7bb77d6086befd9bdcf936479fd246d1065cbd2c6aff55b1d39a67aff965c1>
        let commit_calldata = read_to_bytes("./testdata/calldata_v0.bin")?;
        let blocks = decode_v0(&commit_calldata)?;

        assert_eq!(blocks.data.l2_blocks().len(), 28);

        let last_block = blocks.data.l2_blocks().last().expect("should have 28 blocks");
        let expected_block = L2Block {
            transactions: vec![bytes!(
                "f88c8202418417d7840082a4f294530000000000000000000000000000000000000280a4bede39b50000000000000000000000000000000000000000000000000000000156faa40283104ec3a01339778fe9b41ef708daaa24c455bf93a7b4689863553deb5a508d671556da71a03de900a02261954daee0fd5ed3009984417509f955875784688ae3228a0c5a55"
            )],
            context: BlockContext {
                number: 680,
                timestamp: 1696933798,
                base_fee: U256::ZERO,
                gas_limit: 10000000,
                num_transactions: 1,
                num_l1_messages: 0,
            },
        };

        assert_eq!(last_block, &expected_block);

        Ok(())
    }
}
