use alloy_primitives::{
    bytes::{Buf, BufMut, Bytes, BytesMut},
    PrimitiveSignature,
};
use alloy_rlp::{Decodable, Encodable, RlpDecodable, RlpEncodable};
use reth_eth_wire::{protocol::Protocol, Capability};

/// The message IDs for messages sent over the ScrollWire protocol.
/// This is used to identify the type of message being sent or received
/// and is a requirement for RLPx multiplexing.
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ScrollWireMessageId {
    NewBlock = 0,
}

/// The different kinds of messages that can be sent over the ScrollWire protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScrollWireMessageKind {
    NewBlock(NewBlock),
}

/// A message that is used to announce a new block to the network.
#[derive(Clone, Debug, PartialEq, Eq, RlpEncodable, RlpDecodable)]
pub struct NewBlock {
    pub signature: Bytes,
    pub block: reth_primitives::Block,
}

impl NewBlock {
    pub fn new(signature: PrimitiveSignature, block: reth_primitives::Block) -> Self {
        Self {
            signature: Bytes::from(signature.as_bytes().to_vec()),
            block,
        }
    }
}

impl TryFrom<u8> for ScrollWireMessageId {
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
pub struct ScrollWireMessage {
    pub message_type: ScrollWireMessageId,
    pub message: ScrollWireMessageKind,
}

impl ScrollWireMessage {
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
        Self {
            message_type: ScrollWireMessageId::NewBlock,
            message: ScrollWireMessageKind::NewBlock(block),
        }
    }

    /// Encodes the message into a `BytesMut` buffer.
    pub fn encoded(&self) -> BytesMut {
        let mut buffer = BytesMut::new();
        buffer.put_u8(self.message_type as u8);
        match &self.message {
            ScrollWireMessageKind::NewBlock(new_block) => {
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

        let id: ScrollWireMessageId = buffer[0].try_into().ok()?;
        buffer.advance(1);

        let kind = match id {
            ScrollWireMessageId::NewBlock => {
                let new_block = NewBlock::decode(buffer).ok()?;
                ScrollWireMessageKind::NewBlock(new_block)
            }
        };

        Some(Self {
            message_type: id,
            message: kind,
        })
    }
}
