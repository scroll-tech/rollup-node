use alloy_primitives::{bytes::Buf, Bytes, B256, U256};
use eyre::{bail, eyre};
use futures::{stream::FuturesOrdered, StreamExt};
use reqwest::Client;
use sea_orm::{prelude::*, ActiveValue};
use sea_orm_migration::{prelude::*, seaql_migrations::Relation};
use std::collections::HashMap;

const BLOCK_DATA_CSV: &str = include_str!("../migration-data/block_data.csv");

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut rdr = csv::Reader::from_reader(BLOCK_DATA_CSV.as_bytes());

        let db = manager.get_connection();
        let records: Vec<ActiveModel> =
            rdr.deserialize::<Record>().filter_map(|a| a.ok().map(Into::into)).collect();
        // we ignore the `Failed to find inserted item` error.
        let _ = Entity::insert_many(records).exec(db).await;

        Ok(())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

#[derive(Debug, serde::Deserialize)]
struct Record {
    number: u64,
    hash: B256,
    extra_data: Bytes,
    difficulty: U256,
}

/// This model should match the model at `scroll_db::models::extra_data`.
#[derive(Clone, Debug, DeriveEntityModel)]
#[sea_orm(table_name = "block_data")]
pub struct Model {
    #[sea_orm(primary_key)]
    number: i64,
    hash: Vec<u8>,
    extra_data: Vec<u8>,
    difficulty: Vec<u8>,
}
impl ActiveModelBehavior for ActiveModel {}

impl From<Record> for ActiveModel {
    fn from(value: Record) -> Self {
        Self {
            number: ActiveValue::Set(value.number as i64),
            hash: ActiveValue::Set(value.hash.to_vec()),
            extra_data: ActiveValue::Set(value.extra_data.to_vec()),
            difficulty: ActiveValue::Set(value.difficulty.to_be_bytes_vec()),
        }
    }
}

/// Download the file and decompress the file.
async fn download(url: &str) -> eyre::Result<Vec<HeaderMetadata>> {
    let client = Client::new();
    const CHUNK_SIZE: u64 = 500_000;
    const MAX_TASKS: usize = 16;

    // Get file size and verify range support
    let total_size = get_file_size(&client, url).await?;
    if total_size == 0 {
        bail!("empty file");
    }

    let full_iterations = total_size / CHUNK_SIZE;
    let final_iteration_size = total_size % CHUNK_SIZE;

    let mut buf = vec![];
    let mut headers = vec![];
    let mut index = 0;
    let mut tasks = FuturesOrdered::new();
    let mut decoder = MetadataDecoder::default();

    loop {
        // If we reach final iteration, pull one last time.
        if full_iterations == index {
            // Pull final chunk.
            let mut final_chunk =
                download_chunk(&client, url, index * CHUNK_SIZE, final_iteration_size).await?;
            buf.append(&mut final_chunk);
            // Decode all available data.
            while let Some(data) = decoder.next(&mut buf.as_slice()) {
                headers.push(data)
            }

            break
        }

        // Add extra query tasks if below minimum.
        while tasks.len() < MAX_TASKS {
            let client = &client;
            tasks.push_back(async move {
                download_chunk(&client, url, index * CHUNK_SIZE, CHUNK_SIZE).await
            });
            index += 1;
        }
        // Handle chunks.
        while let Some(output) = tasks.next().await {
            buf.append(&mut output?);
        }
        // Initialize vanity if empty.
        if decoder.vanity.is_empty() {
            let _ = decoder.load_vanity(&mut buf.as_slice());
        }
        // Decode all available data.
        while let Some(data) = decoder.next(&mut buf.as_slice()) {
            headers.push(data)
        }
    }

    Ok(headers)
}

/// Downloads the next chunk of data.
async fn download_chunk(
    client: &Client,
    url: &str,
    start: u64,
    size: u64,
) -> eyre::Result<Vec<u8>> {
    let response = client
        .get(url)
        .header("Range", format!("bytes={}-{}", start, start + size - 1))
        .send()
        .await?;

    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        bail!("failed to fetch chunk");
    }

    let bytes = response.bytes().await?.to_vec();
    Ok(bytes)
}

/// Returns the size in bytes of the file to download.
async fn get_file_size(client: &Client, url: &str) -> eyre::Result<u64> {
    let response = client.head(url).send().await?;
    if response.status().is_success() {
        if let Some(length) = response.headers().get("content-length") {
            let length: u64 = length.to_str()?.parse()?;
            if response.headers().get("accept-ranges").and_then(|h| h.to_str().ok()) ==
                Some("bytes")
            {
                return Ok(length);
            }
        }
    }
    Err(eyre!("server does not support range requests or content-length"))
}

/// The missing metadata for the header.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HeaderMetadata {
    extra_data: Vec<u8>,
    difficulty: u8,
}

/// The metadata decoder. Holds the vanity data.
#[derive(Debug, Clone, Default)]
struct MetadataDecoder {
    vanity: HashMap<u8, [u8; 32]>,
}

impl MetadataDecoder {
    /// Load the vanity data in the decoder.
    fn load_vanity(&mut self, buf: &mut &[u8]) -> eyre::Result<()> {
        // sanity check.
        if buf.is_empty() {
            return Err(eyre!("empty buf"))
        }

        // get the vanity count.
        let vanity_len = buf[0] as usize;
        if buf.len() < 32 * vanity_len {
            return Err(eyre!("missing vanity data"))
        }
        buf.advance(1);

        let mut vanities = HashMap::with_capacity(vanity_len);
        for i in 0..vanity_len {
            vanities.insert(i as u8, buf[..32].try_into().expect("32 bytes slice"));
            buf.advance(32);
        }

        self.vanity = vanities;
        Ok(())
    }

    /// Decodes the next header metadata from the buffer, advancing it.
    fn next(&self, buf: &mut &[u8]) -> Option<HeaderMetadata> {
        // sanity check.
        if buf.len() < 2 {
            return None
        }

        let flag = buf[0];
        let vanity_index = buf[1];
        buf.advance(2);

        let difficulty = if flag & 0b01000000 == 0 { 2 } else { 1 };
        let seal_length = if flag & 0b10000000 == 0 { 65 } else { 85 };

        if buf.len() < seal_length {
            return None
        }
        let seal = &buf[..seal_length];
        let vanity = self.vanity.get(&vanity_index)?;
        let extra_data = [vanity, seal].concat();
        buf.advance(seal_length);

        Some(HeaderMetadata { extra_data, difficulty })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    use alloy_primitives::{bytes, Bytes};

    #[test]
    fn test_should_decode_vanity() {
        let data = Bytes::from_str(&std::fs::read_to_string("./testdata/vanity-test.bin").unwrap())
            .unwrap();
        let mut decoder = MetadataDecoder::default();
        decoder.load_vanity(&mut data.as_ref()).unwrap();
    }

    #[test]
    fn test_should_decode_header() {
        let vanity =
            Bytes::from_str(&std::fs::read_to_string("./testdata/vanity-test.bin").unwrap())
                .unwrap();
        let mut decoder = MetadataDecoder::default();
        decoder.load_vanity(&mut vanity.as_ref()).unwrap();

        let header_data = bytes!("c00048c3f81f3d998b6652900e1c3183736c238fe4290000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000");
        let header = decoder.next(&mut header_data.as_ref()).unwrap();

        let expected_header = HeaderMetadata{ extra_data: bytes!("0x000000000000000000000000000000000000000000000000000000000000000048c3f81f3d998b6652900e1c3183736c238fe4290000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000").to_vec(), difficulty: 1 };
        assert_eq!(header, expected_header)
    }
}
