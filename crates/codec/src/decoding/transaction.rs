use alloy_primitives::{bytes::Buf, Bytes};
use alloy_rlp::Header;

/// A RLP encoded transaction.
#[derive(Debug)]
pub struct Transaction(pub Bytes);

impl Transaction {
    /// Tries to read from the input buffer into the [`Transaction`]. Returns [`None`] if it can't
    /// read the RLP header from the buffer or the buffer length does not cover all the expected
    /// payload data length.
    pub(super) fn try_from_buf(buf: &mut &[u8]) -> Option<Self> {
        // clone the buffer in order to avoid advancing it.
        #[allow(suspicious_double_ref_op)]
        let header = Header::decode(&mut buf.clone()).ok()?;
        let finish = |buf: &mut &[u8], offset: usize| {
            if buf.remaining() < offset {
                return None
            }

            // copy the transaction bytes and advance the buffer.
            let tx = Transaction(buf[0..offset].to_vec().into());
            buf.advance(offset);
            Some(tx)
        };

        if header.list {
            // legacy tx.
            finish(buf, header.length_with_payload())
        } else {
            // typed transaction.
            let mut tx_decode = *buf;
            let _ = tx_decode.get_u8();
            let header = Header::decode(&mut tx_decode).ok()?;

            finish(buf, header.length_with_payload() + 1)
        }
    }
}
