// Copyright 2019-2020 ChainX Project Authors. Licensed under GPL-3.0.

use std::{fmt::Debug, marker::PhantomData};

use codec::{Decode, Encode};
use subxt::{
    balances::{Balances, BalancesEventsDecoder as _},
    system::{System, SystemEventsDecoder as _},
    Store,
};

use crate::runtime::primitives::{Amount, AssetId, Decimals};

// ============================================================================
// Module
// ============================================================================

/// The subset of the `xpallet_system::Trait`.
// #[module]
pub trait XSystem: System + Balances {}

const MODULE: &str = "XSystem";
/// `EventsDecoder` extension trait.
pub trait XSystemEventsDecoder {
    /// Registers this modules types.
    fn with_x_system(&mut self);
}
impl<T: XSystem> XSystemEventsDecoder for subxt::EventsDecoder<T> {
    fn with_x_system(&mut self) {
        self.with_system();
        self.with_balances();
        self.register_type_size::<Amount>("Amount");
        self.register_type_size::<AssetId>("AssetId");
        self.register_type_size::<Decimals>("Decimals");
    }
}

// ============================================================================
// Storage
// ============================================================================

/// NetworkProps field of the `XSystem` module.
#[derive(Clone, Debug, Eq, PartialEq, Encode, Store)]
pub struct NetworkPropsStore<T: XSystem> {
    #[store(returns = NetworkType)]
    /// Runtime marker.
    pub _runtime: PhantomData<T>,
}

/*
impl<T: XSystem> subxt::Store<T> for NetworkPropsStore<T> {
    const MODULE: &'static str = MODULE;
    const FIELD: &'static str = "NetworkProps";
    type Returns = NetworkType;
    fn prefix(
        metadata: &subxt::Metadata,
    ) -> Result<subxt::sp_core::storage::StorageKey, subxt::MetadataError> {
        Ok(metadata
            .module(Self::MODULE)?
            .storage(Self::FIELD)?
            .prefix())
    }
    fn key(
        &self,
        metadata: &subxt::Metadata,
    ) -> Result<subxt::sp_core::storage::StorageKey, subxt::MetadataError> {
        Ok(metadata
            .module(Self::MODULE)?
            .storage(Self::FIELD)?
            .plain()?
            .key())
    }
}

/// Store extension trait.
pub trait NetworkPropsStoreExt<T: subxt::Runtime + XSystem> {
    /// Retrieve the store element.
    fn network_props(
        &self,
        hash: Option<T::Hash>,
    ) -> BoxFuture<'_, Result<NetworkType, subxt::Error>>;
    /// Iterate over the store element.
    fn network_props_iter(
        &self,
        hash: Option<T::Hash>,
    ) -> BoxFuture<'_, Result<subxt::KeyIter<T, NetworkPropsStore<T>>, subxt::Error>>;
}
impl<T: subxt::Runtime + XSystem> NetworkPropsStoreExt<T> for subxt::Client<T> {
    fn network_props(
        &self,
        hash: Option<T::Hash>,
    ) -> BoxFuture<'_, Result<NetworkType, subxt::Error>> {
        let _runtime = core::marker::PhantomData::<T>;
        Box::pin(async move {
            self.fetch_or_default(&NetworkPropsStore { _runtime }, hash)
                .await
        })
    }
    fn network_props_iter(
        &self,
        hash: Option<T::Hash>,
    ) -> BoxFuture<'_, Result<subxt::KeyIter<T, NetworkPropsStore<T>>, subxt::Error>> {
        Box::pin(self.iter(hash))
    }
}
*/

#[derive(Clone, Copy, Debug, Eq, PartialEq, Encode, Decode)]
pub enum NetworkType {
    Mainnet,
    Testnet,
}

impl Default for NetworkType {
    fn default() -> Self {
        NetworkType::Testnet
    }
}

// ============================================================================
// Call
// ============================================================================

// ============================================================================
// Event
// ============================================================================
