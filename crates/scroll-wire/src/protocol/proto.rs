use alloy_primitives::bytes::{Buf, BufMut, Bytes, BytesMut};
use alloy_rlp::{Decodable, Encodable, RlpDecodable, RlpEncodable};
use reth_eth_wire::{protocol::Protocol, Capability};
use secp256k1::ecdsa::Signature;

/// The message IDs for messages sent over the scroll wire protocol.
/// This is used to identify the type of message being sent or received
/// and is a requirement for RLPx multiplexing.
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MessageId {
    NewBlock = 0,
}

/// The different message payloads that can be sent over the ScrollWire protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessagePayload {
    NewBlock(NewBlock),
}

/// A message that is used to announce a new block to the network.
#[derive(Clone, Debug, PartialEq, Eq, RlpEncodable, RlpDecodable)]
pub struct NewBlock {
    /// The signature from the block author.
    pub signature: Bytes,
    /// The block that is being announced.
    pub block: reth_primitives::Block,
}

impl NewBlock {
    /// Returns a [`NewBlock`] instance with the provided signature and block.
    pub fn new(signature: Signature, block: reth_primitives::Block) -> Self {
        Self { signature: Bytes::from(signature.serialize_compact().to_vec()), block }
    }
}

impl TryFrom<u8> for MessageId {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::NewBlock),
            _ => Err(()),
        }
    }
}

/// The ScrollWire message type.
#[derive(Clone, Debug)]
pub struct Message {
    pub id: MessageId,
    pub payload: MessagePayload,
}

impl Message {
    /// Returns the capability of the `ScrollWire` protocol.
    pub const fn capability() -> Capability {
        Capability::new_static("scroll-wire", 1)
    }

    /// Returns the capability of the `ScrollWire` protocol.
    pub const fn protocol() -> Protocol {
        Protocol::new(Self::capability(), 1)
    }

    /// Creates a new block message with the provided signature and block.
    pub fn new_block(block: NewBlock) -> Self {
        Self { id: MessageId::NewBlock, payload: MessagePayload::NewBlock(block) }
    }

    /// Encodes the message into a `BytesMut` buffer.
    pub fn encoded(&self) -> BytesMut {
        let mut buffer = BytesMut::new();
        buffer.put_u8(self.id as u8);
        match &self.payload {
            MessagePayload::NewBlock(new_block) => {
                new_block.encode(&mut buffer);
            }
        }
        buffer
    }

    /// Decodes a message from a `Bytes` buffer.
    pub fn decode(buffer: &mut &[u8]) -> Option<Self> {
        if buffer.is_empty() {
            return None;
        }

        let id: MessageId = buffer[0].try_into().ok()?;
        buffer.advance(1);

        let kind = match id {
            MessageId::NewBlock => {
                let new_block = NewBlock::decode(buffer).ok()?;
                MessagePayload::NewBlock(new_block)
            }
        };

        Some(Self { id, payload: kind })
    }
}
