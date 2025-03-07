use crate::{random, Header};
use arbitrary::Arbitrary;

/// Returns a chain of random block of size `len`, starting at the provided header.
pub fn chain_from(header: &Header, len: usize) -> Vec<Header> {
    if len < 2 {
        panic!("fork should have a minimal length of two");
    }
    let mut blocks = Vec::with_capacity(len);
    blocks.push(header.clone());

    let next_header = |header: &Header| {
        let mut next = random!(Header);
        next.parent_hash = header.hash;
        next.number = header.number + 1;
        next
    };
    for i in 0..len - 1 {
        blocks.push(next_header(&blocks[i]));
    }
    blocks
}
