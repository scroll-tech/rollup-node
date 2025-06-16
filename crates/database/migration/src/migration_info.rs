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
        Some(b256!("9062e2fa1200dca63bee1d18d429572f134f5f0c98cb4852f62fc394e33cf6e6"))
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
        Some(b256!("3629f5e53250a526ffc46806c4d74b9c52c9209a6d45ecdfebdef5d596bb3f40"))
    }
}
