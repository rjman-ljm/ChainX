mod rpc;

use std::marker::PhantomData;

use futures::future;
pub use jsonrpsee::client::Subscription;

use codec::Decode;
pub use sp_core::storage::{StorageChangeSet, StorageKey};
use sp_version::RuntimeVersion;

use self::rpc::Rpc;
pub use self::rpc::{BlockNumber, ExtrinsicSuccess, SystemProperties};
use crate::{
    error::Error,
    events::EventsDecoder,
    metadata::{EncodedCall, Metadata},
    runtime::{
        create_signed, frame::system::SystemStoreExt, Call, Runtime, SignedBlock, Signer, Store,
        UncheckedExtrinsic,
    },
};

/// ClientBuilder for constructing a Client.
#[derive(Default)]
pub struct ClientBuilder<T: Runtime> {
    _runtime: PhantomData<T>,
    url: Option<String>,
    client: Option<jsonrpsee::client::Client>,
}

impl<T: Runtime> ClientBuilder<T> {
    /// Create a new ClientBuilder.
    pub fn new() -> Self {
        Self {
            _runtime: PhantomData,
            url: None,
            client: None,
        }
    }

    /// Set the rpc address url.
    pub fn set_url<P: Into<String>>(mut self, url: P) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Set the jsonrpsee client.
    pub fn set_client<P: Into<jsonrpsee::client::Client>>(mut self, client: P) -> Self {
        self.client = Some(client.into());
        self
    }

    /// Creates a new Client.
    pub async fn build(self) -> Result<Client<T>, Error> {
        let client = if let Some(client) = self.client {
            client
        } else {
            let url = self.url.as_deref().unwrap_or("ws://127.0.0.1:8087");
            assert!(
                url.starts_with("ws://") || url.starts_with("wss://"),
                "the url accepts websocket protocol only"
            );
            jsonrpsee::ws_client(url).await?
        };
        let rpc = Rpc::new(client);

        let (metadata, genesis_hash, runtime_version, properties) = future::join4(
            rpc.metadata(),
            rpc.genesis_hash(),
            rpc.runtime_version(None),
            rpc.system_properties(),
        )
        .await;

        Ok(Client {
            _runtime: PhantomData,
            rpc,
            metadata: metadata?,
            genesis_hash: genesis_hash?,
            properties: properties.unwrap_or_else(|_| Default::default()),
            runtime_version: runtime_version?,
        })
    }
}

/// Client to interface with a substrate node.
pub struct Client<T: Runtime> {
    _runtime: PhantomData<(fn() -> T::Signature, T::Extra)>,
    rpc: Rpc<T>,
    genesis_hash: T::Hash,
    metadata: Metadata,
    properties: SystemProperties,
    runtime_version: RuntimeVersion,
}

impl<T: Runtime> Clone for Client<T> {
    fn clone(&self) -> Self {
        Self {
            _runtime: PhantomData,
            rpc: self.rpc.clone(),
            genesis_hash: self.genesis_hash,
            metadata: self.metadata.clone(),
            properties: self.properties.clone(),
            runtime_version: self.runtime_version.clone(),
        }
    }
}

impl<T: Runtime> Client<T> {
    /// Return the genesis hash.
    pub fn genesis(&self) -> &T::Hash {
        &self.genesis_hash
    }

    /// Return the chain metadata.
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Returns the system properties
    pub fn properties(&self) -> &SystemProperties {
        &self.properties
    }

    /// Get a block hash. By default returns the latest block hash
    pub async fn block_hash(
        &self,
        block_number: Option<BlockNumber>,
    ) -> Result<Option<T::Hash>, Error> {
        self.rpc.block_hash(block_number).await
    }

    /// Fetch a StorageKey with an optional block hash.
    pub async fn fetch<F: Store<T>>(
        &self,
        store: &F,
        hash: Option<T::Hash>,
    ) -> Result<Option<F::Returns>, Error> {
        let key = store.key(&self.metadata)?;
        if let Some(data) = self.rpc.storage(&key, hash).await? {
            Ok(Some(Decode::decode(&mut &data.0[..])?))
        } else {
            Ok(None)
        }
    }

    /// Fetch a StorageKey that has a default value with an optional block hash.
    pub async fn fetch_or_default<F: Store<T>>(
        &self,
        store: &F,
        hash: Option<T::Hash>,
    ) -> Result<F::Returns, Error> {
        if let Some(data) = self.fetch(store, hash).await? {
            Ok(data)
        } else {
            Ok(store.default(&self.metadata)?)
        }
    }

    /// Get a header
    pub async fn header(&self, hash: Option<T::Hash>) -> Result<Option<T::Header>, Error> {
        self.rpc.header(hash).await
    }

    /// Get a block hash of the latest finalized block
    pub async fn finalized_head(&self) -> Result<T::Hash, Error> {
        self.rpc.finalized_head().await
    }

    /// Get a block
    pub async fn block(&self, hash: Option<T::Hash>) -> Result<Option<SignedBlock<T>>, Error> {
        self.rpc.block(hash).await
    }

    /// Subscribe to events.
    pub async fn subscribe_events(&self) -> Result<Subscription<StorageChangeSet<T::Hash>>, Error> {
        self.rpc.subscribe_events().await
    }

    /// Subscribe to storages.
    pub async fn subscribe_storage(
        &self,
        key: StorageKey,
    ) -> Result<Subscription<StorageChangeSet<T::Hash>>, Error> {
        self.rpc.subscribe_storage(key).await
    }

    /// Subscribe to new blocks.
    pub async fn subscribe_blocks(&self) -> Result<Subscription<T::Header>, Error> {
        self.rpc.subscribe_blocks().await
    }

    /// Subscribe to finalized blocks.
    pub async fn subscribe_finalized_blocks(&self) -> Result<Subscription<T::Header>, Error> {
        self.rpc.subscribe_finalized_blocks().await
    }

    // ========================================================================

    /// Encodes a call.
    pub fn encode<C: Call<T>>(&self, call: C) -> Result<EncodedCall, Error> {
        Ok(self
            .metadata()
            .module_with_calls(C::MODULE)
            .and_then(|module| module.call(C::FUNCTION, call))?)
    }

    /// Creates a signed extrinsic.
    pub async fn create_signed<C: Call<T>>(
        &self,
        call: C,
        signer: &dyn Signer<T>,
    ) -> Result<UncheckedExtrinsic<T>, Error> {
        let account_nonce = if let Some(nonce) = signer.nonce() {
            nonce
        } else {
            self.account(signer.account_id(), None).await?.nonce
        };
        let call = self.encode(call)?;
        let signed = create_signed(
            &self.runtime_version,
            self.genesis_hash,
            account_nonce,
            call,
            signer,
        )?;
        Ok(signed)
    }

    /// Create and submit an extrinsic and return corresponding Hash if successful
    pub async fn submit_extrinsic(
        &self,
        extrinsic: UncheckedExtrinsic<T>,
    ) -> Result<T::Hash, Error> {
        self.rpc.submit_extrinsic(extrinsic).await
    }

    /// Create and submit an extrinsic and return corresponding Event if successful
    pub async fn submit_and_watch_extrinsic(
        &self,
        extrinsic: UncheckedExtrinsic<T>,
        decoder: EventsDecoder<T>,
    ) -> Result<ExtrinsicSuccess<T>, Error> {
        self.rpc
            .submit_and_watch_extrinsic(extrinsic, decoder)
            .await
    }

    /// Submits a transaction to the chain.
    pub async fn submit<C: Call<T>>(
        &self,
        call: C,
        signer: &(dyn Signer<T> + Send + Sync),
    ) -> Result<T::Hash, Error> {
        let extrinsic = self.create_signed(call, signer).await?;
        self.submit_extrinsic(extrinsic).await
    }

    /// Submits transaction to the chain and watch for events.
    pub async fn watch<C: Call<T>>(
        &self,
        call: C,
        signer: &(dyn Signer<T> + Send + Sync),
    ) -> Result<ExtrinsicSuccess<T>, Error> {
        let extrinsic = self.create_signed(call, signer).await?;
        let decoder = self.events_decoder::<C>();
        self.submit_and_watch_extrinsic(extrinsic, decoder).await
    }

    /// Returns an events decoder for a call.
    pub fn events_decoder<C: Call<T>>(&self) -> EventsDecoder<T> {
        let metadata = self.metadata().clone();
        let mut decoder = EventsDecoder::new(metadata);
        C::events_decoder(&mut decoder);
        decoder
    }
}
