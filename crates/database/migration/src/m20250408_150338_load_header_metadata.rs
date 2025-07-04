use crate::{migration_info::DataSource, MigrationInfo};
use std::{collections::HashMap, time::Duration};

use alloy_primitives::{bytes::Buf, Address, B256};
use eyre::{bail, eyre};
use futures::{stream::FuturesUnordered, StreamExt};
use indicatif::{ProgressBar, ProgressFinish, ProgressState, ProgressStyle};
use reqwest::Client;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use sea_orm::{prelude::*, ActiveValue};
use sea_orm_migration::{prelude::*, seaql_migrations::Relation};
use sha2::{Digest, Sha256};

pub struct Migration<MI>(pub std::marker::PhantomData<MI>);

impl<MI> MigrationName for Migration<MI> {
    fn name(&self) -> &str {
        sea_orm_migration::util::get_file_stem(file!())
    }
}

#[async_trait::async_trait]
impl<MI: MigrationInfo + Send + Sync> MigrationTrait for Migration<MI> {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        match (MI::data_source(), MI::data_hash()) {
            (Some(DataSource::Url(url)), Some(hash)) => {
                // download data.
                let file = download(&url).await.map_err(|err| DbErr::Custom(err.to_string()))?;
                // verify hash of data.
                verify_data_hash(hash, &file).map_err(|err| DbErr::Custom(err.to_string()))?;

                // decode data and convert to database model.
                let records: Vec<ActiveModel> = decode_to_headers(file)
                    .map_err(|err| DbErr::Custom(err.to_string()))?
                    .into_iter()
                    .enumerate()
                    .map(|(i, h)| (i as i64, h).into())
                    .collect();

                let db = manager.get_connection();

                // batch the insertion to avoid `too many SQL variables` error.
                const MAX_BATCH_SIZE: usize = 3000;
                let mut cursor = 0;
                while cursor < records.len() {
                    let start = cursor;
                    let end = (start + MAX_BATCH_SIZE).min(records.len());
                    Entity::insert_many(records[start..end].to_vec()).exec(db).await?;
                    cursor = end;
                }
            }
            (Some(DataSource::Sql(sql)), _) => {
                manager.get_connection().execute_unprepared(&sql).await?;
            }
            _ => (),
        }

        Ok(())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

/// This model should match the model at `scroll_db::models::extra_data`.
#[derive(Clone, Debug, DeriveEntityModel)]
#[sea_orm(table_name = "block_data")]
pub struct Model {
    #[sea_orm(primary_key)]
    number: i64,
    extra_data: Option<Vec<u8>>,
    state_root: Option<Vec<u8>>,
    coinbase: Option<Vec<u8>>,
    nonce: Option<String>,
    difficulty: Option<i8>,
}
impl ActiveModelBehavior for ActiveModel {}

impl From<(i64, HeaderMetadata)> for ActiveModel {
    fn from((bn, header): (i64, HeaderMetadata)) -> Self {
        Self {
            number: ActiveValue::Set(bn),
            extra_data: ActiveValue::Set(Some(header.extra_data)),
            state_root: ActiveValue::Set(Some(header.state_root.to_vec())),
            coinbase: ActiveValue::Set(header.coinbase.map(|c| c.to_vec())),
            nonce: ActiveValue::Set(header.nonce.map(|x| format!("{x:x}"))),
            difficulty: ActiveValue::Set(Some(header.difficulty as i8)),
        }
    }
}

/// Download the file.
async fn download(url: &str) -> eyre::Result<Vec<u8>> {
    // initialize reqwest client with retry middleware.
    let retry_policy = ExponentialBackoff::builder()
        .retry_bounds(Duration::from_millis(100), Duration::from_secs(5))
        .build_with_max_retries(5);
    let client = ClientBuilder::new(
        Client::builder().timeout(Duration::from_secs(60)).pool_max_idle_per_host(20).build()?,
    )
    .with(RetryTransientMiddleware::new_with_policy(retry_policy))
    .build();

    const CHUNK_SIZE: u64 = 16_000_000;
    const MAX_TASKS: usize = 4;

    // get file size and verify range support.
    let total_size = get_file_size(&client, url).await?;
    if total_size == 0 {
        bail!("empty file");
    }
    let iterations =
        total_size / CHUNK_SIZE + if !total_size.is_multiple_of(CHUNK_SIZE) { 1 } else { 0 };

    // create a progress bar.
    let pb = ProgressBar::new(total_size).
        with_finish(ProgressFinish::AndLeave).
        with_style(
            ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")?
                .with_key("eta", |state: &ProgressState, w: &mut dyn Write| write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap())
                .progress_chars("#>-")
        );
    pb.set_position(0);

    // init variables.
    let mut buf = Vec::with_capacity(iterations as usize);
    let mut index = 0;
    let mut cursor = 0;
    let mut tasks = FuturesUnordered::new();

    loop {
        if index == iterations {
            break
        }
        // add extra query tasks if below minimum.
        while tasks.len() < MAX_TASKS && index < iterations {
            let start = index * CHUNK_SIZE;
            let end = (start + CHUNK_SIZE - 1).min(total_size);
            let client = &client;
            tasks.push(async move { (index, download_chunk(client, url, start, end).await) });
            index += 1;
        }
        // polling chunks.
        while let Some((index, output)) = tasks.next().await {
            let output = output?;

            // advance progress bar.
            cursor += output.len();
            pb.set_position(cursor as u64);

            buf.push((index, output));
            if tasks.is_empty() {
                break
            }
        }
    }

    buf.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
    let buf = buf.into_iter().flat_map(|(_, data)| data).collect();

    Ok(buf)
}

/// Downloads the next chunk of data.
async fn download_chunk(
    client: &ClientWithMiddleware,
    url: &str,
    start: u64,
    end: u64,
) -> eyre::Result<Vec<u8>> {
    let response = client.get(url).header("Range", format!("bytes={start}-{end}")).send().await?;

    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        let err = response.text().await?;
        bail!("failed to fetch chunk: {err}");
    }

    let bytes = response.bytes().await?.to_vec();
    Ok(bytes)
}

/// Returns the size in bytes of the file to download.
async fn get_file_size(client: &ClientWithMiddleware, url: &str) -> eyre::Result<u64> {
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

/// Check the hash of the data.
fn verify_data_hash(expected_data_hash: B256, data: &[u8]) -> eyre::Result<()> {
    let hash = B256::try_from(Sha256::digest(data).as_slice())?;
    if hash != expected_data_hash {
        bail!("corrupted data, expected data to hash to {expected_data_hash}, got {hash}.")
    }

    Ok(())
}

const HEADER_LOWER_SIZE_LIMIT: usize = 1 + 1 + 32 + 65;

/// Decode the data to headers metadata.
fn decode_to_headers(data: Vec<u8>) -> eyre::Result<Vec<HeaderMetadata>> {
    // initialize vanity.
    let mut decoder = MetadataDecoder::default();
    let data_buf = &mut data.as_slice();
    decoder.load_vanity(data_buf).map_err(|err| DbErr::Custom(err.to_string()))?;

    // decode all available data.
    let mut headers = Vec::with_capacity(data_buf.len() / HEADER_LOWER_SIZE_LIMIT);
    while !data_buf.is_empty() {
        headers.push(decoder.next(data_buf).map_err(|err| DbErr::Custom(err.to_string()))?);
    }

    Ok(headers)
}

/// The missing metadata for the header.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HeaderMetadata {
    extra_data: Vec<u8>,
    state_root: Vec<u8>,
    coinbase: Option<Vec<u8>>,
    nonce: Option<u64>,
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
            bail!("empty buf");
        }

        // get the vanity count.
        let vanity_len = buf[0] as usize;
        if buf.len() < 32 * vanity_len {
            bail!("missing vanity data");
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
    fn next(&self, buf: &mut &[u8]) -> eyre::Result<HeaderMetadata> {
        // sanity check.
        if buf.len() < 2 {
            bail!("header buffer too small to read seal flag and vanity index");
        }

        // get flag and vanity index.
        let flag = buf[0];
        let vanity_index = buf[1];

        let has_coinbase = (flag & 0b00010000) != 0;
        let has_nonce = (flag & 0b00100000) != 0;
        let difficulty = if flag & 0b01000000 == 0 { 2 } else { 1 };
        let seal_length = if flag & 0b10000000 == 0 { 65 } else { 85 };
        let vanity = self.vanity.get(&vanity_index).ok_or(eyre!("vanity not found"))?;

        // flag + vanity index + state root + coinbase + nonce + seal
        let total_expected_size = 2 * size_of::<u8>() +
            B256::len_bytes() +
            Address::len_bytes() * has_coinbase as usize +
            size_of::<u64>() * has_nonce as usize +
            seal_length;

        if buf.len() < total_expected_size {
            bail!("header buffer too small: got {}, expected {}", buf.len(), total_expected_size);
        }
        buf.advance(2);

        let state_root = buf[..B256::len_bytes()].to_vec();
        buf.advance(B256::len_bytes());

        let mut coinbase = None;
        if has_coinbase {
            coinbase = Some(buf[..Address::len_bytes()].to_vec());
            buf.advance(Address::len_bytes());
        }

        let mut nonce = None;
        if has_nonce {
            nonce = Some(u64::from_be_bytes(
                buf[..size_of::<u64>()].try_into().expect("32 bytes slice"),
            ));
            buf.advance(size_of::<u64>());
        }

        let seal = &buf[..seal_length];
        let extra_data = [vanity, seal].concat();
        buf.advance(seal_length);

        Ok(HeaderMetadata { extra_data, state_root, coinbase, nonce, difficulty })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    use alloy_primitives::{b256, bytes, Bytes};

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

        let header_data = bytes!("c00020695989e9038823e35f0e88fbc44659ffdbfa1fe89fbeb2689b43f15fa64cb548c3f81f3d998b6652900e1c3183736c238fe4290000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000");
        let header = decoder.next(&mut header_data.as_ref()).unwrap();

        let expected_header = HeaderMetadata {
            extra_data: bytes!("0x000000000000000000000000000000000000000000000000000000000000000048c3f81f3d998b6652900e1c3183736c238fe4290000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000").to_vec(),
            state_root: b256!("20695989e9038823e35f0e88fbc44659ffdbfa1fe89fbeb2689b43f15fa64cb5").to_vec(),
            coinbase: None,
            nonce: None,
            difficulty: 1,
        };
        assert_eq!(header, expected_header)
    }
}
