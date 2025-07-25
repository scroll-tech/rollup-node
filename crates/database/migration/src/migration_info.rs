use alloy_primitives::{b256, B256};

pub enum DataSource {
    Url(String),
    Sql(String),
}

pub trait MigrationInfo {
    fn data_source() -> Option<DataSource>;
    fn data_hash() -> Option<B256>;
}

impl MigrationInfo for () {
    fn data_source() -> Option<DataSource> {
        None
    }

    fn data_hash() -> Option<B256> {
        None
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
}

pub struct ScrollMainnetTestMigrationInfo;

impl MigrationInfo for ScrollMainnetTestMigrationInfo {
    fn data_source() -> Option<DataSource> {
        Some(DataSource::Sql(include_str!(".././testdata/mainnet-sample.sql").into()))
    }

    fn data_hash() -> Option<B256> {
        None
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
}
