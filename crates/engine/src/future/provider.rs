use crate::EngineDriverError;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use alloy_provider::Provider;
use scroll_alloy_network::Scroll;

/// An enum that represents the different types of futures that can be executed on the RPC provider.
pub(crate) enum ProviderFuture {
    BlockNumber {
        fut: Pin<Box<dyn Future<Output = Result<u64, EngineDriverError>> + Send>>,
        current_number: u64,
    },
}

impl ProviderFuture {
    /// Creates a new [`ProviderFuture::BlockNumber`] future.
    pub(crate) fn block_number<P: Provider<Scroll> + 'static>(
        provider: P,
        current_block_number: u64,
    ) -> Self {
        let fut = Box::pin(async move { Ok(provider.get_block_number().await?) });
        Self::BlockNumber { fut, current_number: current_block_number }
    }
}

impl Future for ProviderFuture {
    type Output = ProviderFutureResult;

    /// Polls the [`ProviderFuture`] and upon completion, returns the result of the
    /// corresponding future by converting it into an [`ProviderFutureResult`].
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<ProviderFutureResult> {
        let this = self.get_mut();
        match this {
            Self::BlockNumber { fut, current_number } => {
                fut.as_mut().poll(cx).map(|res| Into::into((res, *current_number)))
            }
        }
    }
}

/// A type that represents the result of the provider future.
#[derive(Debug)]
pub(crate) enum ProviderFutureResult {
    BlockNumber { provider_result: Result<u64, EngineDriverError>, current_number: u64 },
}

impl From<(Result<u64, EngineDriverError>, u64)> for ProviderFutureResult {
    fn from(value: (Result<u64, EngineDriverError>, u64)) -> Self {
        Self::BlockNumber { provider_result: value.0, current_number: value.1 }
    }
}
