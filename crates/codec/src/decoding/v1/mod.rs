pub use batch_header::BatchHeaderV1;
mod batch_header;

use crate::{
    L2Block, check_buf_len,
    decoding::{
        batch::Batch, blob::BlobSliceIter, payload::PayloadData, transaction::Transaction,
        v0::BlockContextV0,
    },
    error::DecodingError,
    from_be_bytes_slice_and_advance_buf,
};
use std::vec::Vec;

use alloy_primitives::bytes::Buf;
use scroll_l1::abi::calls::CommitBatchCall;

/// The max amount of chunks per batch for V1 codec.
/// <https://github.com/scroll-tech/da-codec/blob/main/encoding/codecv0.go#L18>
const MAX_CHUNKS_PER_BATCH: usize = 15;

/// The index offset in the blob to the transaction data.
/// <https://github.com/scroll-tech/da-codec/blob/main/encoding/codecv1.go#L152>
const TRANSACTION_DATA_BLOB_INDEX_OFFSET: usize = 4 * MAX_CHUNKS_PER_BATCH;

/// The block context for v1 decoding is identical to the v0 decoding.
pub(crate) type BlockContextV1 = BlockContextV0;

/// Decodes the input calldata and blob into a [`Batch`].
pub fn decode_v1(calldata: &[u8], blob: &[u8]) -> Result<Batch, DecodingError> {
    // abi decode into a commit batch call
    let call = CommitBatchCall::try_decode(calldata).ok_or(DecodingError::InvalidCalldataFormat)?;

    // decode the parent batch header.
    let raw_parent_header = call.parent_batch_header().ok_or(DecodingError::MissingParentHeader)?;
    let parent_header = BatchHeaderV1::try_from_buf(&mut (&*raw_parent_header))
        .ok_or(DecodingError::InvalidParentHeaderFormat)?;
    let l1_message_start_index = parent_header.total_l1_message_popped;

    let chunks = call.chunks().ok_or(DecodingError::MissingChunkData)?;

    // get blob iterator and collect, skipping unused bytes.
    let heap_blob = BlobSliceIter::from_blob_slice(blob).copied().collect::<Vec<_>>();
    let buf = &mut (heap_blob.as_slice());

    // check buf len.
    check_buf_len!(buf, 2 + TRANSACTION_DATA_BLOB_INDEX_OFFSET);

    // check the chunk count is correct in debug.
    let chunk_count = from_be_bytes_slice_and_advance_buf!(u16, buf);
    debug_assert_eq!(chunks.len(), chunk_count as usize, "mismatched chunk count");

    // move pass chunk information.
    buf.advance(TRANSACTION_DATA_BLOB_INDEX_OFFSET);

    decode_v1_chunk(call.version(), l1_message_start_index, chunks, buf)
}

/// Decode the provided chunks and blob data into [`L2Block`].
pub(crate) fn decode_v1_chunk(
    version: u8,
    l1_message_start_index: u64,
    chunks: Vec<&[u8]>,
    blob: &[u8],
) -> Result<Batch, DecodingError> {
    let mut l2_blocks: Vec<L2Block> = Vec::new();
    let mut chunks_block_count = Vec::new();
    let blob = &mut &*blob;

    // iterate the chunks
    for chunk in chunks {
        let buf: &mut &[u8] = &mut chunk.as_ref();

        // get the block count
        let blocks_count = buf.first().copied().ok_or(DecodingError::Eof)? as usize;
        chunks_block_count.push(blocks_count);
        buf.advance(1);

        let mut block_contexts: Vec<BlockContextV1> = Vec::with_capacity(blocks_count);
        l2_blocks.reserve(blocks_count);

        // for each block, decode into a block context
        for _ in 0..blocks_count {
            let context = BlockContextV1::try_from_buf(buf).ok_or(DecodingError::Eof)?;
            block_contexts.push(context);
        }

        // for each block context, reserve the capacity for the transactions and decode
        // transactions.
        for context in block_contexts {
            let transactions_count = context.transactions_count();
            let mut transactions = Vec::with_capacity(transactions_count);
            for _ in 0..transactions_count {
                let tx = Transaction::try_from_buf(blob).ok_or(DecodingError::Eof)?;
                transactions.push(tx.0);
            }

            l2_blocks.push(L2Block::new(transactions, context.into()))
        }
    }

    let payload =
        PayloadData { blocks: l2_blocks, l1_message_queue_info: l1_message_start_index.into() };

    Ok(Batch::new(version, Some(chunks_block_count), payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlockContext, decoding::test_utils::read_to_bytes};

    use alloy_primitives::{U256, bytes};

    #[test]
    fn test_should_decode_v1() -> eyre::Result<()> {
        // <https://etherscan.io/tx/0x27d73eef6f0de411f8db966f0def9f28c312a0ae5cfb1ac09ec23f8fa18b005b>
        let commit_calldata = read_to_bytes("./testdata/calldata_v1.bin")?;
        let blob = read_to_bytes("./testdata/blob_v1.bin")?;
        let blocks = decode_v1(&commit_calldata, &blob)?;

        assert_eq!(blocks.data.l2_blocks().len(), 12);

        let last_block = blocks.data.l2_blocks().last().expect("should have 12 blocks");
        let expected_block = L2Block {
            transactions: vec![
                bytes!(
                    "f9018d0b840c6aed6a8303c4739403290a52ba3164639067622e20b90857eaded29980b901245a47ddc300000000000000000000000006efdbff2a14a7c8e15944d1f4a48f9f95f663a400000000000000000000000095a52ec1d60e74cd3eb002fe54a2c74b185a4c16000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000004c4b400000000000000000000000000000000000000000000000485a88defc419ee30d0000000000000000000000000000000000000000000000000000000000487ab0000000000000000000000000000000000000000000000044bc686d6fa4bd57b3000000000000000000000000da84471519a3193f1a70b3dc44cbbad95cc1fb00000000000000000000000000000000000000000000000000000000006649bf9483104ec4a0eb1242ce33c73b3a551f21480efbaba9190f79241f880086959a3fca04474d4aa02265c4e0f869b4ac7cb3227b1133d4e133b335aa62b67e8ece0a7d188a89443b"
                ),
                bytes!(
                    "f8d182e39e841260cdb083085ba8946c1bf433a7c8549a61bc7818adfd8fe34083362e80b86700000000000013f5000059bf000004b4006905c59be1a7ea32d1f257e302401ec9a1401c5200000553000000000000000000000000000000000000041053a46826348d67cb8b29f7aeab784240e4d6ba00100206efdbff2a14a7c8e15944d1f4a48f9f95f663a483104ec3a04ecaa4624d8f365974a78b82f2ecd3e6f368b90308bfeec1f35ce07c0aba334ca02a3aebbba7d26cd6d39677baeeea876ef35367396163ad6123f22e191b991f0c"
                ),
                bytes!(
                    "f9016c108410249b09826e8e9447fbe95e981c0df9737b6971b451fb15fdc989d980b901045b7d7482000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000406366613731386630313632613931633437333861316537373466616535656537626130623639343466613936616566366336633230343831393266366339666400000000000000000000000000000000000000000000000000000000000000403861383566396136356162613936393539393935623166646461653837353963613462643235323333383039643539633562323132663039326164626531623483104ec3a0833996f40f3736941d946144c71949425c5b358e3fde33f86332f335b89d1a9da07e9d3e8f842da134c025f992acb57ea79a0a7a8d423eafd8675c666b9a9a5539"
                ),
                bytes!(
                    "f8ec03840f85a8c5830493e09411fcfe756c05ad438e312a7fd934381537d3cffe80b884617ba03700000000000000000000000006efdbff2a14a7c8e15944d1f4a48f9f95f663a400000000000000000000000000000000000000000000000000000000122dee4000000000000000000000000074e5ede2a0b08c1c102d11df230d8f1a35eb8f21000000000000000000000000000000000000000000000000000000000000000083104ec3a0aebc28ae05cb78aa9abf6107ccb72136075f88e357c2d7a482c81b156150fcdda0647ad2146c841bfa9bad81bd56d966422949583cedd6dd20b1586952ec829073"
                ),
                bytes!(
                    "f9020d02840f85a8c58302772f94aaaaaaaacb71bf2c8cae522ea5fa455571a7410680b901a4a15112f9000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000140000000000000000000000000000000000000000000000000000000000000000000000000000000000000000006efdbff2a14a7c8e15944d1f4a48f9f95f663a400000000000000000000000000000000000000000000000000000000000001a400000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000134fd90000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000100010000000000000000000000000000000000000000000000000016da83b8869789000000000000000000000000000000000000000000000000000000000000000083104ec4a04c054fdf0f54ad803831a856e1ce0f6c9a8816e36783f52729137a5c9ebf2adfa048ad4753e75993a0db427b0abef71b982f10a7c173aa6e37857abf148364189c"
                ),
                bytes!(
                    "f8d165840da8d1f5834c4b40940614eb91042383308db5ffd4e3c19376994045d980b86944c0ffee000000000000000000000000663bb50e021d675222304d1c09370a3922f46b63d6024ea768530000000000000000000000000000000000000401a16905c59be1a7ea32d1f257e302401ec9a1401c5206efdbff2a14a7c8e15944d1f4a48f9f95f663a401f083104ec3a04f9cb8b725c77886a85c8c3c4a02f010bfb9858e8c792a78e396368585172397a0188a82b3ac3edc191d1d832a5219f266aa9f6e2ec0947558b44d96dfd09d5d29"
                ),
                bytes!(
                    "f8ab46840da8d1f482eabf941d675222304d1c09370a3922f46b63d6024ea76880b844095ea7b3000000000000000000000000aa111c62cdeef205f70e6722d1e22274274ec12f00000000000000000000000000000000000000000000000000000002061d606083104ec3a021d5137f6238abffd72958720b37dfbc3deca20301aae611ef6049cb7443ee8fa00bc9c3b2dcd45fbda12938de0796a9a808fbe8abeaf3c11d18ace58483cd8a5a"
                ),
                bytes!(
                    "f902cd3b840da8d1f483036b399480e38291e06339d10aab483c65695d004dbd5c6980b902642cc4081e000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000100691070a7ac70000000000000000000000000000000000000000000000000000018f901d948b00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000006000000000000000000000000006efdbff2a14a7c8e15944d1f4a48f9f95f663a40000000000000000000000000000000000000000000000000000000000d8e42300000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000020000000000000000000000000814a23b053fd0f102aeeda0459215c2444799c70000000000000000000000000000000000000000000000000000000000000008000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000006000000000000000000000000006efdbff2a14a7c8e15944d1f4a48f9f95f663a400000000000000000000000092a6bb8be20cbb59cbc51801e2063aa1f2b9a37c0000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000083104ec3a01b9ed91a324058a8c5ed65f2a68d710f4376bb60633b812f6f15507a2b0a9874a05aca21bbee0d77144bc669ece49deb8d99e73b999c36b0a8f90a4f5d50029f46"
                ),
                bytes!(
                    "f8ac28840da8d1f4830778b394ec53c830f4444a8a56455c6836b5d2aa794289aa80b844962941780000000000000000000000000d8f8e271dd3f2fc58e5716d3ff7041dbe3f0688000000000000000000000000000000000000000000000000000000000046227883104ec3a01fde8eea8fa4e323a12747e7b71e5f977e6ce5a767099af6171eba998f763fb9a07a34de8996eac217c372a19976e13dd353d1ccfb2fd809d65a9ff633b96371d6"
                ),
                bytes!(
                    "f9016d08840d29a9bd83031cb89418b71386418a9fca5ae7165e31c385a5130011b680b9010418cbafe500000000000000000000000000000000000000000000000000000000008f6ec0000000000000000000000000000000000000000000000000000a7910af6718d900000000000000000000000000000000000000000000000000000000000000a000000000000000000000000045d94cb8789a527369c59d7bca001647f4687af2000000000000000000000000000000000000000000000000000000006649c183000000000000000000000000000000000000000000000000000000000000000200000000000000000000000006efdbff2a14a7c8e15944d1f4a48f9f95f663a4000000000000000000000000530000000000000000000000000000000000000483104ec4a04f9482373c54d778c17c1c9d002952ee711fbffb193b76d4bd1e583e1ace59fea02a3859cc344fc5f6726cebc6f1ffb0aa1b1702d4bdf79b482352c55f9855bb73"
                ),
            ],
            context: BlockContext {
                number: 5802096,
                timestamp: 1716108620,
                base_fee: U256::ZERO,
                gas_limit: 10000000,
                num_transactions: 10,
                num_l1_messages: 0,
            },
        };

        assert_eq!(last_block, &expected_block);

        Ok(())
    }
}
