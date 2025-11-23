use crate::error::CacheError;

use super::{EthRequestError, L1WatcherResult};

use std::num::NonZeroUsize;

use alloy_primitives::{TxHash, B256};
use alloy_provider::Provider;
use alloy_rpc_types_eth::{Transaction, TransactionTrait};
use lru::LruCache;

/// The L1 watcher cache.
#[derive(Debug)]
pub(crate) struct Cache {
    transaction_cache: TransactionCache,
    // TODO: introduce block cache.
}

impl Cache {
    /// Creates a new [`Cache`] instance with the given capacity for the transaction cache.
    pub(crate) fn new(transaction_cache_capacity: NonZeroUsize) -> Self {
        Self { transaction_cache: TransactionCache::new(transaction_cache_capacity) }
    }

    /// Gets the transaction for the given hash, fetching it from the provider if not cached.
    pub(crate) async fn get_transaction_by_hash<P: Provider>(
        &mut self,
        tx_hash: TxHash,
        provider: &P,
    ) -> L1WatcherResult<Transaction> {
        self.transaction_cache.get_transaction_by_hash(tx_hash, provider).await
    }

    /// Gets the next blob versioned hash for the given transaction hash.
    ///
    /// Errors if the transaction is not in the cache. This method must be called only after
    /// fetching the transaction via [`Self::get_transaction_by_hash`].
    pub(crate) async fn get_transaction_next_blob_versioned_hash(
        &mut self,
        tx_hash: TxHash,
    ) -> L1WatcherResult<Option<B256>> {
        self.transaction_cache.get_transaction_next_blob_versioned_hash(tx_hash).await
    }
}

/// A cache for transactions fetched from the provider.
#[derive(Debug)]
struct TransactionCache {
    cache: LruCache<TxHash, TransactionEntry>,
}

#[derive(Debug)]
struct TransactionEntry {
    transaction: Transaction,
    blob_versioned_hashes: Vec<B256>,
    blob_versioned_hashes_cursor: usize,
}

impl TransactionCache {
    fn new(capacity: NonZeroUsize) -> Self {
        Self { cache: LruCache::new(capacity) }
    }

    async fn get_transaction_by_hash<P: Provider>(
        &mut self,
        tx_hash: TxHash,
        provider: &P,
    ) -> L1WatcherResult<Transaction> {
        if let Some(entry) = self.cache.get(&tx_hash) {
            return Ok(entry.transaction.clone());
        }

        let transaction = provider
            .get_transaction_by_hash(tx_hash)
            .await?
            .ok_or(EthRequestError::MissingTransactionHash(tx_hash))?;
        self.cache.put(tx_hash, transaction.clone().into());
        Ok(transaction)
    }

    async fn get_transaction_next_blob_versioned_hash(
        &mut self,
        tx_hash: TxHash,
    ) -> L1WatcherResult<Option<B256>> {
        if let Some(entry) = self.cache.get_mut(&tx_hash) {
            let blob_versioned_hash =
                entry.blob_versioned_hashes.get(entry.blob_versioned_hashes_cursor).copied();
            entry.blob_versioned_hashes_cursor += 1;
            Ok(blob_versioned_hash)
        } else {
            Err(CacheError::MissingTransactionInCacheForBlobVersionedHash(tx_hash).into())
        }
    }
}

impl From<Transaction> for TransactionEntry {
    fn from(transaction: Transaction) -> Self {
        let blob_versioned_hashes =
            transaction.blob_versioned_hashes().map(|hashes| hashes.to_vec()).unwrap_or_default();

        Self { transaction, blob_versioned_hashes, blob_versioned_hashes_cursor: 0 }
    }
}
