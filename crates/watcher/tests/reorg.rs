//! Integration test of the reorg detection of the L1 watcher.
#![cfg(feature = "test-utils")]

use alloy_rpc_types_eth::Header;
use arbitrary::Arbitrary;
use rand::Rng;
use rollup_node_watcher::{
    random,
    test_utils::{chain::chain_from, provider::MockProvider},
    Block, L1Notification, L1Watcher,
};
use tracing::{subscriber::set_global_default, Level};

fn setup() {
    let sub = tracing_subscriber::FmtSubscriber::builder().with_max_level(Level::TRACE).finish();
    set_global_default(sub).expect("failed to set subscriber");
}

// Generate a set blocks that will be fed to the l1 watcher.
// Every fork_cycle blocks, generates a small reorg.
fn generate_chain_with_reorgs(len: usize, fork_cycle: usize, max_fork_depth: usize) -> Vec<Block> {
    if fork_cycle < 1 {
        panic!("fork cycle should be bigger than 1");
    }
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
        for i in 1..fork_cycle {
            acc.push(next_header(&acc[i - 1]));
        }

        let mut fork = chain_from(&acc.last().unwrap(), rng.random_range(2..max_fork_depth));
        tip = fork.last().unwrap().clone();

        blocks.append(&mut acc);
        blocks.append(&mut fork);

        acc.clear();
    }
    blocks.into_iter().map(|h| Block { header: h, ..Default::default() }).collect()
}

#[tokio::test]
async fn test_reorg_detection() -> eyre::Result<()> {
    // Given
    setup();
    let blocks = generate_chain_with_reorgs(1000, 100, 10);

    // every 30 blocks, skip 5 blocks in latest, creating a gap and forcing the l1 watcher to
    // resync.
    let latest_blocks = blocks
        .iter()
        .cloned()
        .filter(|b| {
            let rem = b.header.number % 30;
            rem < 5
        })
        .collect::<Vec<_>>();

    // finalized blocks should be 64 blocks late.
    let finalized_blocks = std::iter::repeat(latest_blocks.first().clone())
        .filter_map(|b| b.cloned())
        .take(64)
        .chain(latest_blocks.iter().cloned())
        .collect::<Vec<_>>();

    let start = latest_blocks.first().unwrap().header.number;
    let mock_provider = MockProvider::new(
        blocks.clone().into_iter(),
        std::iter::empty(),
        finalized_blocks,
        latest_blocks.clone(),
    );

    // spawn the watcher and verify received notifications are consistent.
    let mut l1_watcher = L1Watcher::spawn(mock_provider, start).await;

    let current_number = latest_blocks.first().unwrap().header.number;
    let current_hash = latest_blocks.first().unwrap().header.hash;

    for block in &latest_blocks[1..] {
        let notification = l1_watcher.recv().await.unwrap();
        // this is a reorg
        if current_number <= block.header.number {
            assert_eq!(notification.as_ref(), &L1Notification::Reorg(block.header.number))
        }
    }

    Ok(())
}
