use alloy_primitives::{b256, B256};
use reth_scroll_chainspec::{SCROLL_MAINNET_GENESIS_HASH, SCROLL_SEPOLIA_GENESIS_HASH};

pub enum DataSource {
    Url(String),
    Sql(String),
}

pub trait MigrationInfo {
    fn data_source() -> Option<DataSource>;
    fn data_hash() -> Option<B256>;
    fn genesis_hash() -> B256;
}

pub struct ScrollDevMigrationInfo;

impl MigrationInfo for ScrollDevMigrationInfo {
    fn data_source() -> Option<DataSource> {
        None
    }

    fn data_hash() -> Option<B256> {
        None
    }

    fn genesis_hash() -> B256 {
        b256!("0xc77ee681dac901672fee660088df30ef11789ec89837123cdc89690ef1fef766")
    }
}

/// The type implementing migration info for Mainnet.
pub struct ScrollMainnetMigrationInfo;

impl MigrationInfo for ScrollMainnetMigrationInfo {
    fn data_source() -> Option<DataSource> {
        Some(DataSource::Url(
            "https://scroll-block-missing-metadata.s3.us-west-2.amazonaws.com/534352.bin".into(),
        ))
    }

    fn data_hash() -> Option<B256> {
        Some(b256!("fa2746026ec9590e37e495cb20046e20a38fd0e7099abd2012640dddf6c88b25"))
    }

    fn genesis_hash() -> B256 {
        SCROLL_MAINNET_GENESIS_HASH
    }
}

pub struct ScrollMainnetTestMigrationInfo;

impl MigrationInfo for ScrollMainnetTestMigrationInfo {
    fn data_source() -> Option<DataSource> {
        Some(DataSource::Sql(include_str!(".././testdata/mainnet-sample.sql").into()))
    }

    fn data_hash() -> Option<B256> {
        None
    }

    fn genesis_hash() -> B256 {
        SCROLL_MAINNET_GENESIS_HASH
    }
}

/// The type implementing migration info for Sepolia.
pub struct ScrollSepoliaMigrationInfo;

impl MigrationInfo for ScrollSepoliaMigrationInfo {
    fn data_source() -> Option<DataSource> {
        Some(DataSource::Url(
            "https://scroll-block-missing-metadata.s3.us-west-2.amazonaws.com/534351.bin".into(),
        ))
    }

    fn data_hash() -> Option<B256> {
        Some(b256!("a02354c12ca0f918bf4768255af9ed13c137db7e56252348f304b17bb4088924"))
    }

    fn genesis_hash() -> B256 {
        SCROLL_SEPOLIA_GENESIS_HASH
    }
}
