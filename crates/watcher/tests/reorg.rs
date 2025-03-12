//! Integration test of the reorg detection of the L1 watcher.
#![cfg(feature = "test-utils")]

use alloy_rpc_types_eth::Header;
use arbitrary::Arbitrary;
use rand::Rng;
use rollup_node_watcher::{
    random, test_utils::provider::MockProvider, Block, L1Notification, L1Watcher,
};
use tracing::{subscriber::set_global_default, Level};

fn setup() {
    let sub = tracing_subscriber::FmtSubscriber::builder().with_max_level(Level::TRACE).finish();
    let _ = set_global_default(sub);
}

// Generate a set blocks that will be fed to the l1 watcher.
// Every fork_cycle blocks, generates a small reorg.
fn generate_chain_with_reorgs(len: usize, fork_cycle: usize, max_fork_depth: usize) -> Vec<Block> {
    assert!(fork_cycle >= 1, "fork cycle should be bigger than 1");

    let mut blocks = Vec::with_capacity(len);
    let mut rng = rand::rng();
    let next_header = |prev: &Header| {
        let mut header = random!(Header);
        header.number = prev.number + 1;
        header.parent_hash = prev.hash;
        header
    };
    let mut tip = random!(Header);
    while blocks.len() < len {
        let mut acc = vec![tip];
        for i in 1..fork_cycle + 1 {
            acc.push(next_header(&acc[i - 1]));
        }

        let depth = rng.random_range(2..max_fork_depth);
        let reorg = &acc[acc.len() - 1 - depth];
        tip = next_header(reorg);

        blocks.append(&mut acc);

        acc.clear();
    }
    blocks.into_iter().map(|h| Block { header: h, ..Default::default() }).collect()
}

#[tokio::test]
async fn test_reorg_detection() -> eyre::Result<()> {
    // Given
    setup();
    let blocks = generate_chain_with_reorgs(1000, 10, 5);
    let latest_blocks = blocks.clone();

    // finalized blocks should be 64 blocks late.
    let mut finalized_blocks = std::iter::repeat(latest_blocks.first())
        .filter_map(|b| b.cloned())
        .take(64)
        .chain(latest_blocks.iter().cloned())
        .collect::<Vec<_>>();
    finalized_blocks.sort_unstable_by(|a, b| a.header.number.cmp(&b.header.number));

    let start = latest_blocks.first().unwrap().header.number;
    let mock_provider = MockProvider::new(
        blocks.clone().into_iter(),
        std::iter::empty(),
        finalized_blocks.clone(),
        latest_blocks.clone(),
    );

    // spawn the watcher and verify received notifications are consistent.
    let mut l1_watcher = L1Watcher::spawn(mock_provider, start).await;

    let mut latest_number = latest_blocks.first().unwrap().header.number;
    let mut finalized_number = finalized_blocks.first().unwrap().header.number;

    for (latest, finalized) in latest_blocks[1..].iter().zip(finalized_blocks[1..].iter()) {
        // check finalized first.
        if finalized_number < finalized.header.number {
            let notification = l1_watcher.recv().await.unwrap();
            assert_eq!(notification.as_ref(), &L1Notification::Finalized(finalized.header.number));
        }

        if latest_number == latest.header.number {
            continue
        }

        // check latest for reorg or new block.
        if latest_number > latest.header.number {
            // reorg
            let notification = l1_watcher.recv().await.unwrap();
            assert!(matches!(notification.as_ref(), L1Notification::Reorg(_)));
            let notification = l1_watcher.recv().await.unwrap();
            assert_eq!(notification.as_ref(), &L1Notification::NewBlock(latest.header.number));
        } else {
            let notification = l1_watcher.recv().await.unwrap();
            assert_eq!(notification.as_ref(), &L1Notification::NewBlock(latest.header.number));
        }

        // update finalized and latest.
        finalized_number = finalized.header.number;
        latest_number = latest.header.number;
    }

    Ok(())
}

#[tokio::test]
async fn test_gap() -> eyre::Result<()> {
    // Given
    setup();
    let mut blocks = vec![random!(Header)];
    for i in 1..1000 {
        let mut next = random!(Header);
        let prev = &blocks[i - 1];
        next.number = prev.number + 1;
        next.parent_hash = prev.hash;
        blocks.push(next);
    }
    let blocks =
        blocks.into_iter().map(|h| Block { header: h, ..Default::default() }).collect::<Vec<_>>();

    // add a gap every 20 blocks.
    let latest_blocks = blocks
        .iter()
        .filter(|b| {
            let rem = b.header.number % 20;
            rem > 5
        })
        .cloned()
        .collect::<Vec<_>>();

    // finalized blocks should be 64 blocks late.
    let mut finalized_blocks = std::iter::repeat(blocks.first())
        .filter_map(|b| b.cloned())
        .take(64)
        .chain(blocks.iter().cloned())
        .collect::<Vec<_>>();
    finalized_blocks.sort_unstable_by(|a, b| a.header.number.cmp(&b.header.number));

    let start = latest_blocks.first().unwrap().header.number;
    let mock_provider = MockProvider::new(
        blocks.clone().into_iter(),
        std::iter::empty(),
        finalized_blocks.clone(),
        latest_blocks.clone(),
    );

    // spawn the watcher and verify received notifications are consistent.
    let mut l1_watcher = L1Watcher::spawn(mock_provider, start).await;

    let mut latest_number = latest_blocks.first().unwrap().header.number;
    let mut finalized_number = finalized_blocks.first().unwrap().header.number;

    for (latest, finalized) in latest_blocks[1..].iter().zip(finalized_blocks[1..].iter()) {
        // check finalized first.
        if finalized_number < finalized.header.number {
            let notification = l1_watcher.recv().await.unwrap();
            assert_eq!(notification.as_ref(), &L1Notification::Finalized(finalized.header.number));
        }

        if latest_number == latest.header.number {
            continue
        }

        let notification = l1_watcher.recv().await.unwrap();
        assert_eq!(notification.as_ref(), &L1Notification::NewBlock(latest.header.number));

        // update finalized and latest.
        finalized_number = finalized.header.number;
        latest_number = latest.header.number;
    }

    Ok(())
}
