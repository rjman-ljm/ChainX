use std::{convert::TryFrom, marker::PhantomData};

use jsonrpsee::{
    client::{Client, Subscription},
    common::Params,
};
use serde::{Deserialize, Serialize};
use serde_json::to_value as to_json_value;

use codec::{Decode, Encode, Error as CodecError};
use frame_metadata::RuntimeMetadataPrefixed;
use sp_core::{
    storage::{StorageChangeSet, StorageData, StorageKey},
    twox_128, Bytes,
};
use sp_rpc::{list::ListOrValue, number::NumberOrHex};
use sp_runtime::traits::Hash;
// use sp_transaction_pool::TransactionStatus;
use sp_version::RuntimeVersion;

use crate::{
    error::Error,
    events::{EventSubscription, EventsDecoder, RawEvent},
    metadata::Metadata,
    runtime::{frame::system::System, Event, Runtime, SignedBlock},
};

/// Wrapper for NumberOrHex to allow custom From impls
#[derive(Serialize)]
pub struct BlockNumber(NumberOrHex);

impl From<NumberOrHex> for BlockNumber {
    fn from(x: NumberOrHex) -> Self {
        BlockNumber(x)
    }
}

impl From<u32> for BlockNumber {
    fn from(x: u32) -> Self {
        NumberOrHex::Number(x.into()).into()
    }
}

/// Possible transaction status events.
///
/// This events are being emitted by `TransactionPool` watchers,
/// which are also exposed over RPC.
///
/// The status events can be grouped based on their kinds as:
/// 1. Entering/Moving within the pool:
///     - `Future`
///     - `Ready`
/// 2. Inside `Ready` queue:
///     - `Broadcast`
/// 3. Leaving the pool:
///     - `InBlock`
///     - `Invalid`
///     - `Usurped`
///     - `Dropped`
/// 4. Re-entering the pool:
///     - `Retracted`
/// 5. Block finalized:
///     - `Finalized`
///     - `FinalityTimeout`
///
/// The events will always be received in the order described above, however
/// there might be cases where transactions alternate between `Future` and `Ready`
/// pool, and are `Broadcast` in the meantime.
///
/// There is also only single event causing the transaction to leave the pool.
/// I.e. only one of the listed ones should be triggered.
///
/// Note that there are conditions that may cause transactions to reappear in the pool.
/// 1. Due to possible forks, the transaction that ends up being in included
/// in one block, may later re-enter the pool or be marked as invalid.
/// 2. Transaction `Dropped` at one point, may later re-enter the pool if some other
/// transactions are removed.
/// 3. `Invalid` transaction may become valid at some point in the future.
/// (Note that runtimes are encouraged to use `UnknownValidity` to inform the pool about
/// such case).
/// 4. `Retracted` transactions might be included in some next block.
///
/// The stream is considered finished only when either `Finalized` or `FinalityTimeout`
/// event is triggered. You are however free to unsubscribe from notifications at any point.
/// The first one will be emitted when the block, in which transaction was included gets
/// finalized. The `FinalityTimeout` event will be emitted when the block did not reach finality
/// within 512 blocks. This either indicates that finality is not available for your chain,
/// or that finality gadget is lagging behind. If you choose to wait for finality longer, you can
/// re-subscribe for a particular transaction hash manually again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransactionStatus<Hash, BlockHash> {
    /// Transaction is part of the future queue.
    Future,
    /// Transaction is part of the ready queue.
    Ready,
    /// The transaction has been broadcast to the given peers.
    Broadcast(Vec<String>),
    /// Transaction has been included in block with given hash.
    InBlock(BlockHash),
    /// The block this transaction was included in has been retracted.
    Retracted(BlockHash),
    /// Maximum number of finality watchers has been reached,
    /// old watchers are being removed.
    FinalityTimeout(BlockHash),
    /// Transaction has been finalized by a finality-gadget, e.g GRANDPA
    Finalized(BlockHash),
    /// Transaction has been replaced in the pool, by another transaction
    /// that provides the same tags. (e.g. same (sender, nonce)).
    Usurped(Hash),
    /// Transaction has been dropped from the pool because of the limit.
    Dropped,
    /// Transaction is no longer valid in the current state.
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// System properties for a Substrate-based runtime
pub struct SystemProperties {
    /// The address format
    pub ss58_format: u8,
    /// The number of digits after the decimal point in the native token
    pub token_decimals: u8,
    /// The symbol of the native token
    pub token_symbol: String,
}

/// Client for substrate rpc interfaces
pub struct Rpc<T: Runtime> {
    _runtime: PhantomData<T>,
    client: Client,
}

impl<T: Runtime> Clone for Rpc<T> {
    fn clone(&self) -> Self {
        Self {
            _runtime: PhantomData,
            client: self.client.clone(),
        }
    }
}

impl<T: Runtime> Rpc<T> {
    pub fn new(client: Client) -> Self {
        Self {
            _runtime: PhantomData,
            client,
        }
    }

    /// Fetch the genesis hash
    pub async fn genesis_hash(&self) -> Result<T::Hash, Error> {
        let block_zero = Some(BlockNumber(NumberOrHex::Number(0)));
        let genesis_hash = self
            .block_hash(block_zero)
            .await?
            .expect("Genesis must exist");
        Ok(genesis_hash)
    }

    /// Get a block hash, returns hash of latest block by default
    pub async fn block_hash(
        &self,
        block_number: Option<BlockNumber>,
    ) -> Result<Option<T::Hash>, Error> {
        let block_number = block_number.map(ListOrValue::Value);
        let params = Params::Array(vec![to_json_value(block_number)?]);
        let hash = self.client.request("chain_getBlockHash", params).await?;
        Ok(hash)
    }

    /// Fetch a storage data by key
    pub async fn storage(
        &self,
        key: &StorageKey,
        hash: Option<T::Hash>,
    ) -> Result<Option<StorageData>, Error> {
        let params = Params::Array(vec![to_json_value(key)?, to_json_value(hash)?]);
        let data = self.client.request("state_getStorage", params).await?;
        log::debug!("state_getStorage {:?}", data);
        Ok(data)
    }

    /// Fetch the metadata
    pub async fn metadata(&self) -> Result<Metadata, Error> {
        let bytes: Bytes = self
            .client
            .request("state_getMetadata", Params::None)
            .await?;
        let meta: RuntimeMetadataPrefixed = Decode::decode(&mut &bytes[..])?;
        let metadata = Metadata::try_from(meta)?;
        Ok(metadata)
    }

    /// Fetch system properties
    pub async fn system_properties(&self) -> Result<SystemProperties, Error> {
        Ok(self
            .client
            .request("system_properties", Params::None)
            .await?)
    }

    /// Get a header
    pub async fn header(&self, hash: Option<T::Hash>) -> Result<Option<T::Header>, Error> {
        let params = Params::Array(vec![to_json_value(hash)?]);
        let header = self.client.request("chain_getHeader", params).await?;
        Ok(header)
    }

    /// Get a block hash of the latest finalized block
    pub async fn finalized_head(&self) -> Result<T::Hash, Error> {
        let hash = self
            .client
            .request("chain_getFinalizedHead", Params::None)
            .await?;
        Ok(hash)
    }

    /// Get a Block
    pub async fn block(&self, hash: Option<T::Hash>) -> Result<Option<SignedBlock<T>>, Error> {
        let params = Params::Array(vec![to_json_value(hash)?]);
        let block = self.client.request("chain_getBlock", params).await?;
        Ok(block)
    }

    /// Fetch the runtime version
    pub async fn runtime_version(&self, at: Option<T::Hash>) -> Result<RuntimeVersion, Error> {
        let params = Params::Array(vec![to_json_value(at)?]);
        let version = self
            .client
            .request("state_getRuntimeVersion", params)
            .await?;
        Ok(version)
    }

    /// Subscribe to system events.
    pub async fn subscribe_events(&self) -> Result<Subscription<StorageChangeSet<T::Hash>>, Error> {
        let mut storage_key = twox_128(b"System").to_vec();
        storage_key.extend(twox_128(b"Events").to_vec());
        log::debug!("Events storage key {:?}", hex::encode(&storage_key));
        self.subscribe_storage(StorageKey(storage_key)).await
    }

    /// Subscribe to storages.
    pub async fn subscribe_storage(
        &self,
        key: StorageKey,
    ) -> Result<Subscription<StorageChangeSet<T::Hash>>, Error> {
        let keys = Some(vec![key]);
        let params = Params::Array(vec![to_json_value(keys)?]);

        let subscription = self
            .client
            .subscribe("state_subscribeStorage", params, "state_unsubscribeStorage")
            .await?;
        Ok(subscription)
    }

    /// Subscribe to blocks.
    pub async fn subscribe_blocks(&self) -> Result<Subscription<T::Header>, Error> {
        let subscription = self
            .client
            .subscribe(
                "chain_subscribeNewHeads",
                Params::None,
                "chain_subscribeNewHeads",
            )
            .await?;

        Ok(subscription)
    }

    /// Subscribe to finalized blocks.
    pub async fn subscribe_finalized_blocks(&self) -> Result<Subscription<T::Header>, Error> {
        let subscription = self
            .client
            .subscribe(
                "chain_subscribeFinalizedHeads",
                Params::None,
                "chain_subscribeFinalizedHeads",
            )
            .await?;
        Ok(subscription)
    }

    /// Create and submit an extrinsic and return corresponding Hash if successful
    pub async fn submit_extrinsic<E: Encode>(&self, extrinsic: E) -> Result<T::Hash, Error> {
        let bytes: Bytes = extrinsic.encode().into();
        let params = Params::Array(vec![to_json_value(bytes)?]);
        let ext_hash = self
            .client
            .request("author_submitExtrinsic", params)
            .await?;
        Ok(ext_hash)
    }

    /// Watch the status of an extrinsic.
    pub async fn watch_extrinsic<E: Encode>(
        &self,
        extrinsic: E,
    ) -> Result<Subscription<TransactionStatus<T::Hash, T::Hash>>, Error> {
        let bytes: Bytes = extrinsic.encode().into();
        let params = Params::Array(vec![to_json_value(bytes)?]);
        let subscription = self
            .client
            .subscribe(
                "author_submitAndWatchExtrinsic",
                params,
                "author_unwatchExtrinsic",
            )
            .await?;
        Ok(subscription)
    }

    // jsonrpsee subscriptions are interminable.
    // Allows `while let status = subscription.next().await {}`
    // Related: https://github.com/paritytech/substrate-subxt/issues/66
    #[allow(irrefutable_let_patterns)]
    /// Create and submit an extrinsic and return corresponding Event if successful
    pub async fn submit_and_watch_extrinsic<E: Encode>(
        &self,
        extrinsic: E,
        decoder: EventsDecoder<T>,
    ) -> Result<ExtrinsicSuccess<T>, Error> {
        let ext_hash = T::Hashing::hash_of(&extrinsic);
        log::info!("Submitting Extrinsic `{:?}`", ext_hash);

        let events_sub = self.subscribe_events().await?;
        let mut ext_sub = self.watch_extrinsic(extrinsic).await?;

        while let status = ext_sub.next().await {
            log::debug!("Received status {:?}", status);
            match status {
                // ignore in progress extrinsic for now
                TransactionStatus::Future
                | TransactionStatus::Ready
                | TransactionStatus::Broadcast(_) => continue,

                TransactionStatus::InBlock(block_hash) => {
                    log::debug!("Fetching block {:?}", block_hash);
                    let block = self.block(Some(block_hash)).await?;
                    return match block {
                        Some(signed_block) => {
                            log::debug!(
                                "Found block {:?}, with {} extrinsics",
                                block_hash,
                                signed_block.block.extrinsics.len()
                            );
                            let ext_index = signed_block
                                .block
                                .extrinsics
                                .iter()
                                .position(|ext| {
                                    let hash = T::Hashing::hash_of(ext);
                                    hash == ext_hash
                                })
                                .ok_or_else(|| {
                                    Error::Other(format!(
                                        "Failed to find Extrinsic with hash {:?}",
                                        ext_hash,
                                    ))
                                })?;

                            // filter block and extrinsic
                            let mut sub = EventSubscription::new(events_sub, decoder);
                            sub.filter_block(block_hash).filter_extrinsic(ext_index);
                            let mut events = vec![];
                            while let Some(event) = sub.next().await {
                                events.push(event?);
                            }
                            Ok(ExtrinsicSuccess {
                                block: block_hash,
                                extrinsic: ext_hash,
                                events,
                            })
                        }
                        None => Err(format!("Failed to find block {:?}", block_hash).into()),
                    };
                }

                TransactionStatus::Invalid => return Err("Extrinsic Invalid".into()),
                TransactionStatus::Usurped(_) => return Err("Extrinsic Usurped".into()),
                TransactionStatus::Dropped => return Err("Extrinsic Dropped".into()),
                TransactionStatus::Retracted(_) => return Err("Extrinsic Retracted".into()),
                // should have made it `InBlock` before either of these
                TransactionStatus::Finalized(_) => return Err("Extrinsic Finalized".into()),
                TransactionStatus::FinalityTimeout(_) => {
                    return Err("Extrinsic FinalityTimeout".into())
                }
            }
        }
        unreachable!()
    }
}

/// Captures data for when an extrinsic is successfully included in a block
#[derive(Debug)]
pub struct ExtrinsicSuccess<T: System> {
    /// Block hash.
    pub block: T::Hash,
    /// Extrinsic hash.
    pub extrinsic: T::Hash,
    /// Raw runtime events, can be decoded by the caller.
    pub events: Vec<RawEvent>,
}

impl<T: System> ExtrinsicSuccess<T> {
    /// Find the Event for the given module/variant, with raw encoded event data.
    /// Returns `None` if the Event is not found.
    pub fn find_event_raw(&self, module: &str, variant: &str) -> Option<&RawEvent> {
        self.events
            .iter()
            .find(|raw| raw.module == module && raw.variant == variant)
    }

    /// Find the Event for the given module/variant, attempting to decode the event data.
    /// Returns `None` if the Event is not found.
    /// Returns `Err` if the data fails to decode into the supplied type.
    pub fn find_event<E: Event<T>>(&self) -> Result<Option<E>, CodecError> {
        if let Some(event) = self.find_event_raw(E::MODULE, E::EVENT) {
            Ok(Some(E::decode(&mut &event.data[..])?))
        } else {
            Ok(None)
        }
    }
}

/*
#[tokio::test]
async fn test_metadata() {
    const CHAINX_WS_URL: &str = "ws://127.0.0.1:8087";
    let client = jsonrpsee::ws_client(CHAINX_WS_URL).await.unwrap();
    let bytes: Bytes = client
        .request("state_getMetadata", Params::None)
        .await.unwrap();
    let meta: RuntimeMetadataPrefixed = Decode::decode(&mut &bytes[..]).unwrap();

    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(format!("{}/meta.json", dir))
        .unwrap();
    serde_json::to_writer_pretty(file, &meta).unwrap();
}
*/
