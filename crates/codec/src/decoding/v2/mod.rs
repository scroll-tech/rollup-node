pub mod zstd;

use crate::{
    check_buf_len,
    decoding::{
        batch::Batch,
        blob::BlobSliceIter,
        v1::{decode_v1_chunk, BatchHeaderV1},
        v2::zstd::decompress_blob_data,
    },
    error::DecodingError,
    from_be_bytes_slice_and_advance_buf,
};
use std::vec::Vec;

use alloy_primitives::bytes::Buf;
use scroll_l1::abi::calls::CommitBatchCall;

/// The max amount of chunks per batch for V2 codec.
/// <https://github.com/scroll-tech/da-codec/blob/main/encoding/codecv2.go#L25>
const MAX_CHUNKS_PER_BATCH: usize = 45;

/// The index offset in the blob to the transaction data.
/// <https://github.com/scroll-tech/da-codec/blob/main/encoding/codecv1.go#L152>
pub(crate) const TRANSACTION_DATA_BLOB_INDEX_OFFSET: usize = 4 * MAX_CHUNKS_PER_BATCH;

/// Decodes the input calldata and blob into a [`Batch`].
pub fn decode_v2(calldata: &[u8], blob: &[u8]) -> Result<Batch, DecodingError> {
    // abi decode into a commit batch call
    let call = CommitBatchCall::try_decode(calldata).ok_or(DecodingError::InvalidCalldataFormat)?;
    let chunks = call.chunks().ok_or(DecodingError::MissingChunkData)?;

    // decode the parent batch header.
    let raw_parent_header = call.parent_batch_header().ok_or(DecodingError::MissingParentHeader)?;
    let parent_header = BatchHeaderV1::try_from_buf(&mut (&*raw_parent_header))
        .ok_or(DecodingError::InvalidParentHeaderFormat)?;
    let l1_message_start_index = parent_header.total_l1_message_popped;

    // get blob iterator and collect, skipping unused bytes.
    let compressed_heap_blob = BlobSliceIter::from_blob_slice(blob).copied().collect::<Vec<_>>();
    let uncompressed_heap_blob = decompress_blob_data(&compressed_heap_blob);
    let buf = &mut (uncompressed_heap_blob.as_slice());

    // check buf len.
    check_buf_len!(buf, 2 + TRANSACTION_DATA_BLOB_INDEX_OFFSET);

    // check the chunk count is correct in debug.
    let chunk_count = from_be_bytes_slice_and_advance_buf!(u16, buf);
    debug_assert_eq!(chunks.len(), chunk_count as usize, "mismatched chunk count");

    // clone buf and move pass chunk information.
    buf.advance(TRANSACTION_DATA_BLOB_INDEX_OFFSET);

    decode_v1_chunk(call.version(), l1_message_start_index, chunks, buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decoding::test_utils::read_to_bytes, BlockContext, L2Block};

    use alloy_primitives::{bytes, U256};

    #[test]
    fn test_should_decode_v2() -> eyre::Result<()> {
        // <https://etherscan.io/tx/0x6e846b5666670b1b1c87faa54b760be8717c71b1e7e91d5f6c5f20d3d67ee5d8>
        let commit_calldata = read_to_bytes("./testdata/calldata_v2.bin")?;
        let blob = read_to_bytes("./testdata/blob_v2.bin")?;
        let blocks = decode_v2(&commit_calldata, &blob)?;

        assert_eq!(blocks.data.l2_blocks().len(), 47);

        let last_block = blocks.data.l2_blocks().last().expect("should have 47 blocks");
        let expected_block = L2Block {
            transactions: vec![
                bytes!(
                    "02f891830827500e840d1cef00840d1cef0082ab1a94e6feca764b7548127672c189d303eb956c3ba37280a4e95a644f000000000000000000000000000000000000000000000000000000000134d946c001a01ebf717b27fe6086d3996f8858045c5acb054d0cbe1fb643ec5e84f843cf6162a062505fb3eb50bc0bbd4a65cea94c3ad33f257c337edc868e2190781f237e4350"
                ),
                bytes!(
                    "0xf86b30840c7103c98302a43b94ed3ae48d051e1b8eece2b4afaa485641398f598480841249c58b83104ec3a0df0132bbe831b1e4ee6ae64b50e726cde50668a969eacd371df459fedad6293ca0138a017d3fd12d5d5f94cf4ee4ecb6022cfde2de6b561f44260a97dc9df00f10"
                ),
                bytes!(
                    "0xf901c529841583b933830fdb308080b90170608060405234801561001057600080fd5b50610150806100206000396000f3fe608060405234801561001057600080fd5b50600436106100365760003560e01c806352efd6851461003b5780636ca5b5b014610057575b600080fd5b610055600480360381019061005091906100c3565b610075565b005b61005f61007f565b60405161006c91906100ff565b60405180910390f35b8060008190555050565b60008054905090565b600080fd5b6000819050919050565b6100a08161008d565b81146100ab57600080fd5b50565b6000813590506100bd81610097565b92915050565b6000602082840312156100d9576100d8610088565b5b60006100e7848285016100ae565b91505092915050565b6100f98161008d565b82525050565b600060208201905061011460008301846100f0565b9291505056fea2646970667358221220b6b3d0fb7481749a994c784eb7b07dfa8e8f2513429976c82f6b5b6f2b54058f64736f6c6343000812003383104ec4a0c269e20742174bc55122da4de98b263152fdfae54c5ad7793e1c25bf988aea4ca035a7d3fdf751619d10dc24fcd17e9b65d50c384a89933e5a896f3da9df6f14ba"
                ),
                bytes!(
                    "0x02f902bb830827501d840b4f77ce840b4f77ce8304aae994c0ac932cac7b4d8f7c31792082e2e8f3cfe99c10871ff973cafa8000b90244ac9650d800000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000001c0000000000000000000000000000000000000000000000000000000000000014488d527dc000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000c0000000000000000000000000eb1abcf8e1bcfb7e29b9f8b0c5e9fe9a0810938a000000000000000000000000000000000000000000000000001ff973cafa80000000000000000000000000000000000000000000000000000000000001a02d4f000000000000000000000000000000000000000000000000000000000000001900000000000000000000000000000000000000000000000000000000668df40c000000000000000000000000000000000000000000000000000000000000002b53000000000000000000000000000000000000040001f406efdbff2a14a7c8e15944d1f4a48f9f95f663a400000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000412210e8a00000000000000000000000000000000000000000000000000000000c001a0aa025dc24c46fe42a31d44769dced750876185295dde7ea65b01adf8480e6322a07e9a2f227775efd70655696ea88a9897a7308fa9ecea5193cef1e47b9ebbdfc0"
                ),
                bytes!(
                    "0xf90310830b6dd08418968a848305a6b494b87591d8b0b93fae8b631a073577c40e8dd46a6280b902a4b143044b00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000d60000000000000000000000008363302080e711e0cab978c081b9e69308d4980800000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000668e461e00000000000000000000000000000000000000000000000000000000000001c000000000000000000000000000000000000000000000000000000000000000e40223536e0000000000000000000000000000000000000000000000000000000000000060acb39c21ebd48b28e6e8e76f1b6b1ff84c57c742a022ee28f868102f6828d62d000000000000000000000000000000000000000000000000000000000000001400000000000000000000000000000000000000000000000000000000000000510100000000000034d90000759e00000000000000000000000019cfce47ed54a88614648dc3f19a5980097007dd000076060000000000000000000000004e422b0acb2bd7e3ac70b5c0e5eb806e86a940380000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000411899d15049e726d9a920a1195b30c4e05d65ccc83dc56a88ec96fa435cfaf7741de5e97ae069e6b8bf837943f9b7dde1e12a4285dce135bc9e8248323f9ee58a1c0000000000000000000000000000000000000000000000000000000000000083104ec3a01c79fcc6fbb08d8b3cfd18cd65c967f2726cbd6359b426deb31d5c972e84b450a01fada7e74dbd7fdbf80cc1b3e2ab3b7c5355a6b3a74a7d13e32dcdc540263b59"
                ),
            ],
            context: BlockContext {
                number: 7291474,
                timestamp: 1720578497,
                base_fee: U256::from(189757389),
                gas_limit: 10000000,
                num_transactions: 5,
                num_l1_messages: 0,
            },
        };

        assert_eq!(last_block, &expected_block);

        Ok(())
    }
}
