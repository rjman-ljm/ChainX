pub mod extra;
pub mod frame;
pub mod impls;
pub mod signer;

use codec::{Decode, Encode};
use sp_core::storage::StorageKey;
use sp_runtime::{
    generic,
    traits::{SignedExtension, Verify},
};
use sp_version::RuntimeVersion;

use self::frame::system::System;
use crate::{
    error::Error,
    events::EventsDecoder,
    metadata::{EncodedCall, Metadata, MetadataError},
};

/// Store trait.
pub trait Store<T>: Encode {
    /// Module name.
    const MODULE: &'static str;
    /// Field name.
    const FIELD: &'static str;

    /// Return type.
    type Returns: Decode;

    /// Returns the key prefix for storage maps
    fn prefix(metadata: &Metadata) -> Result<StorageKey, MetadataError>;

    /// Returns the `StorageKey`.
    fn key(&self, metadata: &Metadata) -> Result<StorageKey, MetadataError>;

    /// Returns the default value.
    fn default(&self, metadata: &Metadata) -> Result<Self::Returns, MetadataError> {
        metadata
            .module(Self::MODULE)?
            .storage(Self::FIELD)?
            .default()
    }
}

/// Call trait.
pub trait Call<T>: Encode {
    /// Module name.
    const MODULE: &'static str;
    /// Function name.
    const FUNCTION: &'static str;

    /// Load event decoder.
    fn events_decoder(_decoder: &mut EventsDecoder<T>) {}
}

/// Event trait.
pub trait Event<T>: Decode {
    /// Module name.
    const MODULE: &'static str;
    /// Event name.
    const EVENT: &'static str;
}

// ============================================================================

/// Runtime trait.
pub trait Runtime: System + Sized + Send + Sync {
    /// Signature type.
    type Signature: Verify + Encode + Send + Sync;

    /// Transaction extras.
    type Extra: SignedExtra<Self>;
}

/// Trait for implementing transaction extras for a runtime.
pub trait SignedExtra<T: System>: SignedExtension {
    /// The type the extras.
    type Extra: SignedExtension;

    /// Creates a new `SignedExtra`.
    fn new(spec_version: u32, tx_version: u32, nonce: T::Index, genesis_hash: T::Hash) -> Self;

    /// Returns the transaction extra.
    fn extra(&self) -> Self::Extra;
}

/// Extrinsic signer.
pub trait Signer<T: Runtime>: Send + Sync {
    /// Returns the account id.
    fn account_id(&self) -> &T::AccountId;

    /// Optionally returns a nonce.
    fn nonce(&self) -> Option<T::Index>;

    /// Takes an unsigned extrinsic and returns a signed extrinsic.
    ///
    /// Some signers may fail, for instance because the hardware on which the keys are located has
    /// refused the operation.
    fn sign(&self, extrinsic: SignedPayload<T>) -> Result<UncheckedExtrinsic<T>, String>;
}

// ============================================================================

/// SignedBlock type
pub type SignedBlock<T> =
    generic::SignedBlock<generic::Block<<T as System>::Header, <T as System>::Extrinsic>>;

/// Extra type.
pub type Extra<T> = <<T as Runtime>::Extra as SignedExtra<T>>::Extra;

/// UncheckedExtrinsic type.
pub type UncheckedExtrinsic<T> = generic::UncheckedExtrinsic<
    <T as System>::Address,
    EncodedCall,
    <T as Runtime>::Signature,
    Extra<T>,
>;

/// SignedPayload type.
pub type SignedPayload<T> = generic::SignedPayload<EncodedCall, Extra<T>>;

/// Creates an unsigned extrinsic
pub fn create_unsigned<T: Runtime>(call: EncodedCall) -> UncheckedExtrinsic<T> {
    UncheckedExtrinsic::<T>::new_unsigned(call)
}

/// Creates a signed extrinsic
pub fn create_signed<T: Runtime>(
    runtime_version: &RuntimeVersion,
    genesis_hash: T::Hash,
    nonce: T::Index,
    call: EncodedCall,
    signer: &dyn Signer<T>,
) -> Result<UncheckedExtrinsic<T>, Error> {
    let spec_version = runtime_version.spec_version;
    let tx_version = runtime_version.transaction_version;

    let extra: T::Extra = T::Extra::new(spec_version, tx_version, nonce, genesis_hash);
    let payload = SignedPayload::<T>::new(call, extra.extra())?;
    let signed = signer.sign(payload);
    signed.map_err(Error::Other)
}
