//! Shared JSON-RPC helpers for debug toolkit REPLs.

use alloy_provider::{Provider, RootProvider};
use scroll_alloy_network::Scroll;
use std::borrow::Cow;

/// Call a typed JSON-RPC method and deserialize into `R`.
pub(crate) async fn raw_typed<R: serde::de::DeserializeOwned>(
    provider: &RootProvider<Scroll>,
    method: &'static str,
    params: impl serde::Serialize,
) -> eyre::Result<R> {
    let raw_params = serde_json::value::to_raw_value(&params)
        .map_err(|e| eyre::eyre!("Failed to serialize params for {}: {}", method, e))?;
    let raw_result = provider
        .raw_request_dyn(Cow::Borrowed(method), &raw_params)
        .await
        .map_err(|e| eyre::eyre!("{}: {}", method, e))?;
    serde_json::from_str(raw_result.get())
        .map_err(|e| eyre::eyre!("Failed to deserialize response from {}: {}", method, e))
}

/// Call any JSON-RPC method and return the response as a JSON value.
pub(crate) async fn raw_value(
    provider: &RootProvider<Scroll>,
    method: &str,
    params: Option<&str>,
) -> eyre::Result<serde_json::Value> {
    let raw_params = match params {
        None => serde_json::value::to_raw_value(&())?,
        Some(p) => {
            let value: serde_json::Value = serde_json::from_str(p)
                .unwrap_or_else(|_| serde_json::Value::String(p.to_string()));
            let array =
                if value.is_array() { value } else { serde_json::Value::Array(vec![value]) };
            serde_json::value::to_raw_value(&array)?
        }
    };

    let result = provider
        .raw_request_dyn(Cow::Owned(method.to_string()), &raw_params)
        .await
        .map_err(|e| eyre::eyre!("{}: {}", method, e))?;
    serde_json::from_str(result.get())
        .map_err(|e| eyre::eyre!("Failed to deserialize response from {}: {}", method, e))
}
