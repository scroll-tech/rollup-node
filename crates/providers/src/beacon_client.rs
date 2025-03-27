//! Contains an implementation of a Beacon client.
//! Credit to <https://github.com/op-rs/kona/tree/main/crates/providers/providers-alloy>

use alloy_rpc_types_beacon::sidecar::{BeaconBlobBundle, BlobData};
use reqwest::Client;
use std::{format, vec::Vec};

/// The config spec engine api method.
const SPEC_METHOD: &str = "eth/v1/config/spec";

/// The beacon genesis engine api method.
const GENESIS_METHOD: &str = "eth/v1/beacon/genesis";

/// The blob sidecars engine api method prefix.
const SIDECARS_METHOD_PREFIX: &str = "eth/v1/beacon/blob_sidecars";

/// An API response.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct APIResponse<T> {
    /// The data.
    pub data: T,
}

/// A reduced genesis data.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReducedGenesisData {
    /// The genesis time.
    #[serde(rename = "genesis_time")]
    #[serde(with = "alloy_serde::quantity")]
    pub genesis_time: u64,
}

/// A reduced config data.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReducedConfigData {
    /// The seconds per slot.
    #[serde(rename = "SECONDS_PER_SLOT")]
    #[serde(with = "alloy_serde::quantity")]
    pub seconds_per_slot: u64,
}

/// An online implementation of the [BeaconClient] trait.
#[derive(Debug, Clone)]
pub struct OnlineBeaconClient {
    /// The base URL of the beacon API.
    pub base: String,
    /// The inner reqwest client.
    pub inner: Client,
}

impl OnlineBeaconClient {
    /// Creates a new [OnlineBeaconClient] from the provided [Url].
    pub fn new_http(mut base: String) -> Self {
        // If base ends with a slash, remove it
        if base.ends_with("/") {
            base.remove(base.len() - 1);
        }
        Self { base, inner: Client::new() }
    }
}

impl OnlineBeaconClient {
    /// Returns the reduced configuration data for the Beacon client.
    pub async fn config_spec(&self) -> Result<APIResponse<ReducedConfigData>, reqwest::Error> {
        let first = self.inner.get(format!("{}/{}", self.base, SPEC_METHOD)).send().await?;
        first.json::<APIResponse<ReducedConfigData>>().await
    }

    /// Returns the Beacon genesis information.
    pub async fn beacon_genesis(&self) -> Result<APIResponse<ReducedGenesisData>, reqwest::Error> {
        let first = self.inner.get(format!("{}/{}", self.base, GENESIS_METHOD)).send().await?;
        first.json::<APIResponse<ReducedGenesisData>>().await
    }

    /// Returns the blobs for the provided slot.
    pub async fn blobs(&self, slot: u64) -> Result<Vec<BlobData>, reqwest::Error> {
        let raw_response = self
            .inner
            .get(format!("{}/{}/{}", self.base, SIDECARS_METHOD_PREFIX, slot))
            .send()
            .await?;
        let raw_response = raw_response.json::<BeaconBlobBundle>().await?;

        Ok(raw_response.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // <https://docs.arbitrum.io/run-arbitrum-node/l1-ethereum-beacon-chain-rpc-providers>
    const BEACON_CLIENT_URL: &str = "https://eth-beacon-chain.drpc.org/rest/";

    #[tokio::test]
    async fn test_should_return_genesis() -> eyre::Result<()> {
        let client = OnlineBeaconClient::new_http(BEACON_CLIENT_URL.to_string());
        let genesis = client.beacon_genesis().await?;

        assert_eq!(genesis.data.genesis_time, 1606824023);

        Ok(())
    }

    #[tokio::test]
    async fn test_should_return_config() -> eyre::Result<()> {
        let client = OnlineBeaconClient::new_http(BEACON_CLIENT_URL.to_string());
        let config = client.config_spec().await?;

        assert_eq!(config.data.seconds_per_slot, 12);

        Ok(())
    }
}
