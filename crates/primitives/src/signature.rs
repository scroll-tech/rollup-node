use alloy_consensus::Header;
use alloy_primitives::{
    keccak256,
    private::{alloy_rlp, alloy_rlp::Encodable},
    B256, U256,
};

/// Encode and hash the header for signature. The function is similar to `Header::encode` but skips
/// the `extra_data` field.
pub fn sig_encode_hash(header: &Header) -> B256 {
    let out = &mut Vec::new();
    let list_header =
        alloy_rlp::Header { list: true, payload_length: sig_header_payload_length(header) };
    list_header.encode(out);
    header.parent_hash.encode(out);
    header.ommers_hash.encode(out);
    header.beneficiary.encode(out);
    header.state_root.encode(out);
    header.transactions_root.encode(out);
    header.receipts_root.encode(out);
    header.logs_bloom.encode(out);
    header.difficulty.encode(out);
    U256::from(header.number).encode(out);
    U256::from(header.gas_limit).encode(out);
    U256::from(header.gas_used).encode(out);
    header.timestamp.encode(out);
    header.mix_hash.encode(out);
    header.nonce.encode(out);

    // Encode all the fork specific fields
    if let Some(ref base_fee) = header.base_fee_per_gas {
        U256::from(*base_fee).encode(out);
    }
    keccak256(&out)
}

/// Returns the header payload length for signature.
fn sig_header_payload_length(header: &Header) -> usize {
    let mut length = 0;
    length += header.parent_hash.length();
    length += header.ommers_hash.length();
    length += header.beneficiary.length();
    length += header.state_root.length();
    length += header.transactions_root.length();
    length += header.receipts_root.length();
    length += header.logs_bloom.length();
    length += header.difficulty.length();
    length += U256::from(header.number).length();
    length += U256::from(header.gas_limit).length();
    length += U256::from(header.gas_used).length();
    length += header.timestamp.length();
    length += header.mix_hash.length();
    length += header.nonce.length();

    if let Some(base_fee) = header.base_fee_per_gas {
        // Adding base fee length if it exists.
        length += U256::from(base_fee).length();
    }
    length
}
