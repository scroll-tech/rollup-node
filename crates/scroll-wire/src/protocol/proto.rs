use alloy_primitives::{
    bytes::{Buf, BufMut, Bytes, BytesMut},
    Signature,
};
use alloy_rlp::{Decodable, Encodable, RlpDecodable, RlpEncodable};
use reth_eth_wire::{protocol::Protocol, Capability};

/// The message IDs for messages sent over the scroll wire protocol.
/// This is used to identify the type of message being sent or received
/// and is a requirement for `RLPx` multiplexing.
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ScrollMessageId {
    NewBlock = 0,
}

/// The different message payloads that can be sent over the `ScrollWire` protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScrollMessagePayload {
    NewBlock(NewBlock),
}

/// A message that is used to announce a new block to the network.
#[derive(Clone, Debug, PartialEq, Eq, RlpEncodable, RlpDecodable)]
pub struct NewBlock {
    /// The signature from the block author.
    pub signature: Bytes,
    /// The block that is being announced.
    pub block: reth_scroll_primitives::ScrollBlock,
}

impl NewBlock {
    /// Returns a [`NewBlock`] instance with the provided signature and blocks.
    pub fn new(signature: Signature, block: reth_scroll_primitives::ScrollBlock) -> Self {
        Self { signature: Bytes::from(Into::<Vec<u8>>::into(signature)), block }
    }
}

impl TryFrom<u8> for ScrollMessageId {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::NewBlock),
            _ => Err(()),
        }
    }
}

/// The scroll wire message type.
#[derive(Clone, Debug)]
pub struct ScrollMessage {
    pub id: ScrollMessageId,
    pub payload: ScrollMessagePayload,
}

impl ScrollMessage {
    /// Returns the capability of the scroll wire protocol.
    pub const fn capability() -> Capability {
        Capability::new_static("scroll", 1)
    }

    /// Returns the capability of the scroll wire protocol.
    pub const fn protocol() -> Protocol {
        Protocol::new(Self::capability(), 1)
    }

    /// Creates a new block message with the provided signature and block.
    pub const fn new_block(block: NewBlock) -> Self {
        Self { id: ScrollMessageId::NewBlock, payload: ScrollMessagePayload::NewBlock(block) }
    }

    /// Encodes the message into a [`BytesMut`] buffer.
    pub fn encoded(&self) -> BytesMut {
        let mut buffer = BytesMut::new();
        buffer.put_u8(self.id as u8);
        match &self.payload {
            ScrollMessagePayload::NewBlock(new_block) => {
                new_block.encode(&mut buffer);
            }
        }
        buffer
    }

    /// Decodes a message from a bytes buffer.
    pub fn decode(buffer: &mut &[u8]) -> Option<Self> {
        if buffer.is_empty() {
            return None;
        }

        let id: ScrollMessageId = buffer[0].try_into().ok()?;
        buffer.advance(1);

        let kind = match id {
            ScrollMessageId::NewBlock => {
                let new_block = NewBlock::decode(buffer).ok()?;
                ScrollMessagePayload::NewBlock(new_block)
            }
        };

        Some(Self { id, payload: kind })
    }
}
