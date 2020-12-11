use std::{fmt::Debug, hash::Hash as StdHash, marker::PhantomData, str::FromStr};

use codec::{Decode, Encode, FullCodec};
use frame_support::{
    dispatch::{DispatchError, DispatchInfo},
    Parameter,
};
use futures::future::BoxFuture;
use sp_core::storage::StorageKey;
use sp_runtime::traits::{
    AtLeast32Bit, AtLeast32BitUnsigned, Bounded, CheckEqual, Extrinsic, Hash, Header, MaybeDisplay,
    MaybeMallocSizeOf, MaybeSerialize, MaybeSerializeDeserialize, Member, SimpleBitOps,
};

use crate::{
    client::{Client, ExtrinsicSuccess},
    error::Error,
    events::EventsDecoder,
    metadata::{Metadata, MetadataError},
    runtime::{Event, Runtime, Store},
};

// ============================================================================
// Module
// ============================================================================

/// The subset of the `frame::Trait` that must implement.
pub trait System: Eq + Clone {
    /// Account index (aka nonce) type. This stores the number of previous transactions associated
    /// with a sender account.
    type Index: Parameter
        + Member
        + MaybeSerialize
        + Debug
        + Default
        + MaybeDisplay
        + AtLeast32Bit
        + Copy;

    /// The block number type used by the runtime.
    type BlockNumber: Parameter
        + Member
        + MaybeSerializeDeserialize
        + Debug
        + MaybeDisplay
        + AtLeast32BitUnsigned
        + Default
        + Bounded
        + Copy
        + StdHash
        + FromStr
        + MaybeMallocSizeOf;

    /// The output of the `Hashing` function.
    type Hash: Parameter
        + Member
        + MaybeSerializeDeserialize
        + Debug
        + MaybeDisplay
        + SimpleBitOps
        + Ord
        + Default
        + Copy
        + CheckEqual
        + StdHash
        + AsRef<[u8]>
        + AsMut<[u8]>
        + MaybeMallocSizeOf;

    /// The hashing system (algorithm) being used in the runtime (e.g. Blake2).
    type Hashing: Hash<Output = Self::Hash>;

    /// The user account identifier type for the runtime.
    type AccountId: Parameter
        + Member
        + MaybeSerializeDeserialize
        + Debug
        + MaybeDisplay
        + Ord
        + Default;

    /*
    /// Converting trait to take a source type and convert to `AccountId`.
    ///
    /// Used to define the type and conversion mechanism for referencing accounts in transactions.
    /// It's perfectly reasonable for this to be an identity conversion (with the source type being
    /// `AccountId`), but other modules (e.g. Indices module) may provide more functional/efficient
    /// alternatives.
    type Lookup: StaticLookup<Target = Self::AccountId>;
    */
    /// The address type. This instead of `<frame_system::Trait::Lookup as StaticLookup>::Source`.
    type Address: Parameter + Member;

    /// The block header.
    type Header: Parameter
        + MaybeSerializeDeserialize
        + Header<Number = Self::BlockNumber, Hash = Self::Hash>;

    /// Extrinsic type within blocks.
    type Extrinsic: Parameter + Member + MaybeSerializeDeserialize + Extrinsic;

    /// Data to be associated with an account (other than nonce/transaction counter, which this
    /// module does regardless).
    type AccountData: Member + FullCodec + Clone + Default;
}

pub const MODULE: &str = "System";

/// `EventsDecoder` extension trait.
pub trait SystemEventsDecoder {
    /// Registers this modules types.
    fn with_system(&mut self);
}

impl<T: System> SystemEventsDecoder for EventsDecoder<T> {
    fn with_system(&mut self) {
        self.register_type_size::<T::Index>("Index");
        self.register_type_size::<T::BlockNumber>("BlockNumber");
        self.register_type_size::<T::Hash>("Hash");
        self.register_type_size::<T::AccountId>("AccountId");
        self.register_type_size::<T::AccountData>("AccountData");
    }
}

/// A phase of a block's execution.
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode)]
pub enum Phase {
    /// Applying an extrinsic.
    ApplyExtrinsic(u32),
    /// Finalizing the block.
    Finalization,
    /// Initializing the block.
    Initialization,
}

impl Default for Phase {
    fn default() -> Self {
        Self::Initialization
    }
}

// ============================================================================
// Storage
// ============================================================================

pub trait SystemStoreExt<T: System> {
    fn account<'a>(
        &'a self,
        account_id: &'a T::AccountId,
        hash: Option<T::Hash>,
    ) -> BoxFuture<'a, Result<AccountInfo<T>, Error>>;
}

impl<T: Runtime + System> SystemStoreExt<T> for Client<T> {
    fn account<'a>(
        &'a self,
        account_id: &'a T::AccountId,
        hash: Option<T::Hash>,
    ) -> BoxFuture<'a, Result<AccountInfo<T>, Error>> {
        Box::pin(async move {
            self.fetch_or_default(&AccountStore { account_id }, hash)
                .await
        })
    }
}

/// Account field of the `System` module.
#[derive(Clone, Debug, Eq, PartialEq, Encode)]
pub struct AccountStore<'a, T: System> {
    /// Account to retrieve the `AccountInfo<T>` for.
    pub account_id: &'a T::AccountId,
}

impl<'a, T: System> Store<T> for AccountStore<'a, T> {
    const MODULE: &'static str = MODULE;
    const FIELD: &'static str = "Account";

    type Returns = AccountInfo<T>;

    fn prefix(metadata: &Metadata) -> Result<StorageKey, MetadataError> {
        Ok(metadata
            .module(Self::MODULE)?
            .storage(Self::FIELD)?
            .prefix())
    }

    fn key(&self, metadata: &Metadata) -> Result<StorageKey, MetadataError> {
        Ok(metadata
            .module(Self::MODULE)?
            .storage(Self::FIELD)?
            .map()?
            .key(&self.account_id))
    }
}

/// Type used to encode the number of references an account has.
pub type RefCount = u32;

/// Information of an account.
#[derive(Clone, Debug, Eq, PartialEq, Default, Encode, Decode)]
pub struct AccountInfo<T: System> {
    /// The number of transactions this account has sent.
    pub nonce: T::Index,
    /// The number of other modules that currently depend on this account's existence. The account
    /// cannot be reaped until this is zero.
    pub refcount: RefCount,
    /// The additional data that belongs to this account. Used to store the balance(s) in a lot of
    /// chains.
    pub data: T::AccountData,
}

// ============================================================================
// Call
// ============================================================================

// ============================================================================
// Event
// ============================================================================

/// Event extension trait.
pub trait SystemEventExt<T: System> {
    fn extrinsic_success(&self) -> Result<Option<ExtrinsicSuccessEvent<T>>, codec::Error>;

    fn extrinsic_failed(&self) -> Result<Option<ExtrinsicFailedEvent<T>>, codec::Error>;
}

impl<T: System> SystemEventExt<T> for ExtrinsicSuccess<T> {
    fn extrinsic_success(&self) -> Result<Option<ExtrinsicSuccessEvent<T>>, codec::Error> {
        self.find_event()
    }

    fn extrinsic_failed(&self) -> Result<Option<ExtrinsicFailedEvent<T>>, codec::Error> {
        self.find_event()
    }
}

/// An extrinsic completed successfully.
#[derive(Clone, Debug, Eq, PartialEq, Decode)]
pub struct ExtrinsicSuccessEvent<T: System> {
    /// Runtime marker.
    pub _runtime: PhantomData<T>,
    /// The dispatch info.
    pub info: DispatchInfo,
}

impl<T: System> Event<T> for ExtrinsicSuccessEvent<T> {
    const MODULE: &'static str = MODULE;
    const EVENT: &'static str = "ExtrinsicSuccess";
}

/// An extrinsic failed.
#[derive(Clone, Debug, Eq, PartialEq, Decode)]
pub struct ExtrinsicFailedEvent<T: System> {
    /// Runtime marker.
    pub _runtime: PhantomData<T>,
    /// The dispatch error.
    pub error: DispatchError,
    /// The dispatch info.
    pub info: DispatchInfo,
}

impl<T: System> Event<T> for ExtrinsicFailedEvent<T> {
    const MODULE: &'static str = MODULE;
    const EVENT: &'static str = "ExtrinsicFailed";
}
