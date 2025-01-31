use alloy_primitives::{Address, B256};
use alloy_rpc_types_engine::PayloadAttributes;
use arbitrary::Unstructured;

// impl<'a> arbitrary::Arbitrary<'a> for ScrollPayloadAttributes {
//     fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
//         Ok(Self {
//             payload_attributes: PayloadAttributes {
//                 timestamp: u64::arbitrary(u)?,
//                 prev_randao: B256::arbitrary(u)?,
//                 suggested_fee_recipient: Address::arbitrary(u)?,
//                 withdrawals: Some(Vec::arbitrary(u)?),
//                 parent_beacon_block_root: Some(B256::arbitrary(u)?),
//             },
//             transactions: Some(Vec::arbitrary(u)?),
//             no_tx_pool: bool::arbitrary(u)?,
//         })
//     }
// }
