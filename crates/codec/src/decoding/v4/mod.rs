use crate::{
    L2Block, check_buf_len,
    decoding::{blob::BlobSliceIter, v1::decode_v1_chunk, v2::zstd::decompress_blob_data},
    error::DecodingError,
    from_be_bytes_slice_and_advance_buf,
};
use std::vec::Vec;

use alloy_primitives::bytes::Buf;
use alloy_sol_types::SolCall;
use scroll_l1::abi::calls::commitBatchWithBlobProofCall;

/// Decodes the input calldata and blob into a [`Vec<L2Block>`].
pub fn decode_v4(calldata: &[u8], blob: &[u8]) -> Result<Vec<L2Block>, DecodingError> {
    // abi decode into a commit batch call
    let call = commitBatchWithBlobProofCall::abi_decode(calldata, true)
        .map_err(|_| DecodingError::InvalidCalldataFormat)?;

    // get blob iterator and collect, skipping unused bytes.
    let mut heap_blob = BlobSliceIter::from_blob_slice(blob).copied().collect::<Vec<_>>();

    // check for compression.
    let is_compressed = *heap_blob.first().ok_or(DecodingError::Eof)?;
    debug_assert!(is_compressed == 1 || is_compressed == 0, "incorrect compressed byte flag");

    let buf = if is_compressed == 1 {
        heap_blob = decompress_blob_data(&heap_blob[1..]);
        &mut heap_blob.as_slice()
    } else {
        &mut (&heap_blob[1..])
    };

    check_buf_len!(buf, 2 + super::v2::TRANSACTION_DATA_BLOB_INDEX_OFFSET);

    // check the chunk count is correct in debug.
    let chunk_count = from_be_bytes_slice_and_advance_buf!(u16, buf);
    debug_assert_eq!(call.chunks.len(), chunk_count as usize, "mismatched chunk count");

    // clone buf and move pass chunk information.
    buf.advance(super::v2::TRANSACTION_DATA_BLOB_INDEX_OFFSET);

    decode_v1_chunk(call.chunks, buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlockContext, decoding::test_utils::read_to_bytes};

    use alloy_primitives::{U256, bytes};

    #[test]
    fn test_should_decode_v4_uncompressed() -> eyre::Result<()> {
        // <https://etherscan.io/tx/0x27d73eef6f0de411f8db966f0def9f28c312a0ae5cfb1ac09ec23f8fa18b005b>
        let commit_calldata = read_to_bytes("./src/testdata/calldata_v4_uncompressed.bin")?;
        let blob = read_to_bytes("./src/testdata/blob_v4_uncompressed.bin")?;
        let blocks = decode_v4(&commit_calldata, &blob)?;

        assert_eq!(blocks.len(), 12);

        let last_block = blocks.last().expect("should have 12 blocks");
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
                num_l1_messages: 0,
            },
        };

        assert_eq!(last_block, &expected_block);

        Ok(())
    }

    #[test]
    fn test_should_decode_v4_compressed() -> eyre::Result<()> {
        // <https://etherscan.io/tx/0xee0afe29207fe23626387bc8eb209ab751c1fee9c18e3d6ec7a5edbcb5a4fed4>
        let commit_calldata = read_to_bytes("./src/testdata/calldata_v4_compressed.bin")?;
        let blob = read_to_bytes("./src/testdata/blob_v4_compressed.bin")?;
        let blocks = decode_v4(&commit_calldata, &blob)?;

        assert_eq!(blocks.len(), 47);

        let last_block = blocks.last().expect("should have 47 blocks");
        let expected_block = L2Block {
            transactions: vec![
                bytes!(
                    "02f9017a830827501d8402c15db28404220c8b833bf0fa94d6238ad2887166031567616d9a54b21eb70e4dfd865af3107a4000b901042f73d60a000000000000000000000000000000000000000000000000000000000000008000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000005af3107a400000000000000000000000000000000000000000000000000000000000000000640000000000000000000000000000000000000000000000000000000000000005727465727400000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000045254525400000000000000000000000000000000000000000000000000000000c080a01ab3cf2a93857170eb1a8a564a00dc54d9dbc081aff236614c05f00f89564e7ea076143846b8e83dbbedc9f7f39d9e1efafd2aa323af5977acbc3b7559eaa61338"
                ),
                bytes!(
                    "02f90213830827505d830ebf5b8403c6fdd68303160094aaaaaaaacb71bf2c8cae522ea5fa455571a7410680b901a4a15112f900000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000014000000000000000000000000006efdbff2a14a7c8e15944d1f4a48f9f95f663a4000000000000000000000000ca77eb3fefe3725dc33bccb54edefc3d9f764f9700000000000000000000000000000000000000000000000000000000000001a40000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001a96557e8b05a2e0dd00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000010001000000000000000000000000000000000000000000000000000000001ce571940000000000000000000000000000000000000000000000000000000000000000c001a038385859bdc661006ee04173ef0c5e7d259f213b38ec65c5ac5664cc2263588aa06edfcce7499e39f78ff336265222272f75e3b8b6292bc5e7a9b785ec2764357f"
                ),
                bytes!(
                    "f901d43c84039387008301eb0694dc3d8318fbaec2de49281843f5bba22e78338146870110d9316ec000b901647c2ccc45000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000e00000000000000000000000006d1aa44dfe55c66e2dd413b045aaf3db92e8bf920000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000004108690eca0b490e1a9ebaf85710cce8dd72d48eeb6e74f03fcf1ea58638afbe8808c5076a512e0993e77b117e938d8505bd41380209374a5fa1736040386f9c7a1c0000000000000000000000000000000000000000000000000000000000000083104ec3a00a9daf43e323158d459652563edb141a0df3f2b6d890f6307aac52c74e0bbbbfa02163e491f0cfbeec828ccf98b8877db9ba2a6552e18b2d4d3a8c5ded1d407d73"
                ),
                bytes!(
                    "f8948201ef8402faf08083018a31940241fb446d6793866245b936f2c3418f818bdcd3879970b65dfdc000a4b6b55f250000000000000000000000000000000000000000000000000098c445ad57800083104ec4a097f352f786ffb1ddf9d942286cbd9ff6839f46093767c4326a1cf9bc1f117500a048cfccfe406c692cc717a670a0148fef79789aceb282cc3d0e6805593ad605cf"
                ),
                bytes!(
                    "f90bd45c8402faf080830e3b1994a2a9fd768d482caf519d749d3123a133db278a66876a94d74f42ffffb90b645973bd5e000000000000000000000000eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee000000000000000000000000ca77eb3fefe3725dc33bccb54edefc3d9f764f97000000000000000000000000000000000000000000000000006a94d74f42ffff000000000000000000000000000000000000000000000003e3fdab75eefcae8200000000000000000000000083412753e54768f8bed921e5556680e7a3e1910800000000000000000000000000000000000000000000000000000000000000e00000000000000000000000000000000000000000000000000000000066d85f8c0000000000000000000000000000000000000000000000000000000000000a600000000000000000000000006131b5fae19ea4f9d964eac0408e4408b66337b5000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000009e4e21fd0e90000000000000000000000000000000000000000000000000000000000000020000000000000000000000000f40442e1cb0bdfb496e8b7405d0c1c48a81bc897000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000004e000000000000000000000000000000000000000000000000000000000000007600000000000000000000000000000000000000000000000000000000000000420000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000c00000000000000000000000005300000000000000000000000000000000000004000000000000000000000000ca77eb3fefe3725dc33bccb54edefc3d9f764f970000000000000000000000006131b5fae19ea4f9d964eac0408e4408b66337b50000000000000000000000000000000000000000000000000000000066d85f8c00000000000000000000000000000000000000000000000000000000000003c00000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000016000000000000000000000000000000000000000000000000000000000000000401b96cfd40000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000c00000000000000000000000008f8ed95b3b3ed2979d1ee528f38ca3e481a94dd9000000000000000000000000530000000000000000000000000000000000000400000000000000000000000006efdbff2a14a7c8e15944d1f4a48f9f95f663a4000000000000000000000000f40442e1cb0bdfb496e8b7405d0c1c48a81bc897000000000000000000000000000000000000000000000000006a94d74f42ffff0000000000000000000000000000000000000000000000000000000000030f0b000000000000000000000000000000000000000000000000000000000000004063407a490000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000e0000000000000000000000000f40442e1cb0bdfb496e8b7405d0c1c48a81bc897000000000000000000000000ccdf79ced5fd02af299d3548b4e35ed6163064bf00000000000000000000000006efdbff2a14a7c8e15944d1f4a48f9f95f663a4000000000000000000000000ca77eb3fefe3725dc33bccb54edefc3d9f764f9700000000000000000000000000000000000000000000000000000000044a4e800000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000200000000000000000000041783c314dc20000000000000003e6fce47750c3ecea0000000000000000000000005300000000000000000000000000000000000004000000000000000000000000ca77eb3fefe3725dc33bccb54edefc3d9f764f97000000000000000000000000000000000000000000000000000000000000016000000000000000000000000000000000000000000000000000000000000001a000000000000000000000000000000000000000000000000000000000000001e00000000000000000000000000000000000000000000000000000000000000220000000000000000000000000e7a23e2f9abf813ad55e55ce26c0712bf1593332000000000000000000000000000000000000000000000000006a94d74f42ffff000000000000000000000000000000000000000000000003da07eeddb6d6509a00000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000002600000000000000000000000000000000000000000000000000000000000000001000000000000000000000000f40442e1cb0bdfb496e8b7405d0c1c48a81bc8970000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000006a94d74f42ffff0000000000000000000000000000000000000000000000000000000000000001000000000000000000000000a8337cce66f217701071a68a503caa8bf139b1840000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000001e000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002317b22536f75726365223a22686970706f2d73776170222c22416d6f756e74496e555344223a2237322e323133343430313133383636222c22416d6f756e744f7574555344223a2237312e3931343239373937333238373232222c22526566657272616c223a22222c22466c616773223a302c22416d6f756e744f7574223a223731373638373037373539383535313532373730222c2254696d657374616d70223a313732353435353036382c22496e74656772697479496e666f223a7b224b65794944223a2231222c225369676e6174757265223a22436d38564441314d3466386b696f525546383976695344495765474e785858726c3842643651396a70493057764c652b6f4d655246394472484e73544c463045315a5842736e55384b4a393173693447674631524d74614d334c777030324f5136383879704e796e436b3978425a4b2b796b427074416647614b35516a49794f45712f36494d4a654d3772626f59444675713166414f7370394634683543714c44622f7469722b507562677131474b693742556a6d6433584463796239386a70377a5132783533744e52766e52683955484f44636932516252634a6e337272394157327252397653697a4b46336874676c546b794a6a61725251446e735644772f3274687447595a4e7a516a417361354b717236323679796e466f49493175387779714b547a6b3052512f6f464d4a7a55752b454563704d752b4c626a7835322f50556c6735526678666568507066666a4a53514a773d3d227d7d0000000000000000000000000000000000000000000000000000000000000000000000000000000000000083104ec3a0aa2e2380709f2b0c6b8fcc48ba4c6942aea501a867d2bfe27a5979a9900b9692a044a21df53a177fff5c1348b3cdb23f82bab41b8fea58d68138234a302d87d904"
                ),
                bytes!(
                    "f88d8201078403938700830100a794e6feca764b7548127672c189d303eb956c3ba37280a4e95a644f000000000000000000000000000000000000000000000000000000000134da0883104ec3a054336f213352ee4faf92a752befcfe39c7a6a18ce3d7bcc56b6dc875454d76dba00e9ebd78c3aa468f4d311e3ebfeb0c8c79a1f3cfc3a26ed71d3665fc6820b5d5"
                ),
                bytes!(
                    "02f8b28308275024830ebf5b8403c6fdd6830415a894ec53c830f4444a8a56455c6836b5d2aa794289aa80b844830cbbbd000000000000000000000000274c3795dadfebf562932992bf241ae087e0a98c00000000000000000000000000000000000000000000000021b745fecb550714c080a0e8f444aca5c459c27676e185579de1d6cb5eb88d4e350fd6dade07946ede16a8a0165e301acebf43a608e747e07b2e32ddd86cebb24ec98440c34b919c49fcedd7"
                ),
                bytes!(
                    "02f9025483082750518403db832c8403db832c8308bcf394c47300428b6ad2c7d03bb76d05a176058b47e6b080b901e4f17325e70000000000000000000000000000000000000000000000000000000000000020d57de4f41c3d3cc855eadef68f98c0d4edd22d57161d96b7c06d2f4336cc3b490000000000000000000000000000000000000000000000000000000000000040000000000000000000000000295f5db3e40c5155271eaf9058d54b185c5fff1300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000002dbce60ebeaafb77e5472308f432f78ac3ae07d90000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000004000000000000000000000000074670a3998d9d6622e32d0847ff5977c37e0ec91000000000000000000000000000000000000000000000000000000000004e900c080a0f63bb59e762a76fe9252065519362897d63795bedffe88fc922a683c82e7e8d0a07693bc90d37b2922650f8e065a0daac6ea71af62c83b4fa69285e7c511785b7a"
                ),
                bytes!(
                    "f902ae819a8403ef1480830ae583940b4d5229bb5201e277d3937ce1704227c96bbc5f80b902443c0427150000000000000000000000000000000000000000000000000000000000000020d57de4f41c3d3cc855eadef68f98c0d4edd22d57161d96b7c06d2f4336cc3b4900000000000000000000000000000000000000000000000000000000000000e0000000000000000000000000000000000000000000000000000000000000001bd63bc394b1e44a60f0d5ea4fbb61937d973ddcd80b241370f7939607494853112eb2c36e5e4a5d7b9184961380642680541125dfdd9c0764b7a2efad85f926c30000000000000000000000001f4a828ff025fa8270bfd1d4d952e75079bb593d0000000000000000000000000000000000000000000000000000000066d868f00000000000000000000000005b0d7cfaf6557f026591fc29b8f050d7537b476400000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000600000000000000000000000000c5d859d4bb0963c8f946d3b3751e4976165b38e0000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000000083104ec3a088eb2685bd6b79ae008d6b0ee74b09341d79f590c8b3faea5dfa2237644bc716a01482d3425fa581941b057a4d20960a90f20ce9e43fb425e537d565be705b3e10"
                ),
                bytes!(
                    "02f8768308275083018b348398968085012d5fe6308275309409dcae886c35e45f2545c0087725e36e18b032eb865af3107a43ea80c001a0922e3023fc0a04bb29ec74efecebb381535ef2453907b101b342f8254fa73072a04b5c97606536ab8d9b7ffec96fab415729c53d18b5ff12f5a4fd148f4aa42d15"
                ),
                bytes!(
                    "02f8b1830827505e830ebf5b8403c6fdd682d9c494e97c507e2b88ab55c61d528f506e13e35dcb8f1580b844a22cb4650000000000000000000000000cab6977a9c70e04458b740476b498b2140196410000000000000000000000000000000000000000000000000000000000000001c080a0dab66684749d0773893d7cce976dc4a8d2182db8a59044cd9bbd4d0d64f432a1a01bfe61ca3ceae4a81c3668369c36a0eda672f45d63e3935b13f2a694611da6d6"
                ),
                bytes!(
                    "02f902138308275059830ebf5b8403c6fdd68306cff794c47300428b6ad2c7d03bb76d05a176058b47e6b080b901a4f17325e70000000000000000000000000000000000000000000000000000000000000020d57de4f41c3d3cc855eadef68f98c0d4edd22d57161d96b7c06d2f4336cc3b490000000000000000000000000000000000000000000000000000000000000040000000000000000000000000dcde5e9d35d5a1fe9e0eb3185459b3323e09b73b00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000600000000000000000000000001121f46b5581b5285bc571703cd772b336aa12e600000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000000c001a054dc43a6d710d99764add84512d022b4961dd39bfe366967acec0d39a5ab820fa050362c7b199a5bae81b1b56b2bb70058f64a70a01044821232b2c11aef78250d"
                ),
                bytes!(
                    "02f8b983082750288402e577518402f49919830309f194ec53c830f4444a8a56455c6836b5d2aa794289aa86d8bcb85faa78b844f2b9fdb8000000000000000000000000274c3795dadfebf562932992bf241ae087e0a98c0000000000000000000000000000000000000000000000000000d8bcb85faa78c080a0b12f8b14c62254ac38a2e0f567fa67c642ed485c43d9f2db2a1f93ada8173c86a0178e3a021c50b57fc6f949201d2e1667a243ba0ea94dbf3199abeecea6c20194"
                ),
                bytes!(
                    "f88c81a78402faf080830100a794e6feca764b7548127672c189d303eb956c3ba37280a4e95a644f000000000000000000000000000000000000000000000000000000000134da0883104ec3a059393dff7dc95d7e2f74053268bc1ab67e03ea0aa115929cd31912fca57d8c66a07330a31a06c957e6b4b31eee08a37c5dc15e31d1a37d4284475adc0a248fa920"
                ),
            ],
            context: BlockContext {
                number: 8990862,
                timestamp: 1725455077,
                base_fee: U256::from(46226864),
                gas_limit: 10000000,
                num_l1_messages: 0,
            },
        };

        assert_eq!(last_block, &expected_block);

        Ok(())
    }
}
