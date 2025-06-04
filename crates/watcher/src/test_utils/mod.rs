use crate::{random, Header};
use arbitrary::Arbitrary;

/// Test utils for arbitrary.
pub mod arbitrary;

/// Test utils for provider.
pub mod provider;

/// Returns a chain of random headers of size `len`.
pub fn chain(len: usize) -> (Header, Header, Vec<Header>) {
    assert!(len >= 2, "chain should have a minimal length of two");

    let mut chain = Vec::with_capacity(len);
    chain.push(random!(Header));
    for i in 1..len {
        let mut next = random!(Header);
        next.number = chain[i - 1].number + 1;
        next.parent_hash = chain[i - 1].hash;
        chain.push(next);
    }

    (chain.first().unwrap().clone(), chain.last().unwrap().clone(), chain)
}

/// Returns a chain of random block of size `len`, starting at the provided header.
pub fn chain_from(header: &Header, len: usize) -> Vec<Header> {
    assert!(len >= 2, "fork should have a minimal length of two");

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
