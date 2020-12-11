use std::{fmt::Debug, marker::PhantomData};

use codec::{Decode, Encode};
use pallet_indices::address::Address;
use sp_core::H256;
use sp_runtime::{
    generic::{Era, Header},
    traits::{BlakeTwo256, IdentifyAccount, SignedExtension, Verify},
    transaction_validity::TransactionValidityError,
    MultiSignature, OpaqueExtrinsic,
};

use crate::runtime::{
    extra::{
        ChargeTransactionPayment, CheckEra, CheckGenesis, CheckNonce, CheckSpecVersion,
        CheckTxVersion, CheckWeight,
    },
    frame::{
        balances::{AccountData, Balances},
        system::System,
    },
    Runtime, SignedExtra,
};

/// Concrete type definitions compatible with those in the default substrate `node_runtime`
///
/// # Note
///
/// If the concrete types in the target substrate runtime differ from these, a custom Runtime
/// definition MUST be used to ensure type compatibility.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DefaultNodeRuntime;

impl Runtime for DefaultNodeRuntime {
    type Signature = MultiSignature;
    type Extra = DefaultExtra<Self>;
}

impl System for DefaultNodeRuntime {
    type Index = u32;
    type BlockNumber = u32;
    type Hash = H256;
    type Hashing = BlakeTwo256;
    type AccountId = <<MultiSignature as Verify>::Signer as IdentifyAccount>::AccountId;
    type Address = Address<Self::AccountId, u32>;
    type Header = Header<Self::BlockNumber, Self::Hashing>;
    type Extrinsic = OpaqueExtrinsic;
    type AccountData = AccountData<<Self as Balances>::Balance>;
}

impl Balances for DefaultNodeRuntime {
    type Balance = u128;
}

/// Default `SignedExtra` for substrate runtimes.
#[derive(Encode, Decode, Clone, Eq, PartialEq, Debug)]
pub struct DefaultExtra<T: System> {
    spec_version: u32,
    tx_version: u32,
    nonce: T::Index,
    genesis_hash: T::Hash,
}

impl<T: System + Balances + Debug + Send + Sync> SignedExtra<T> for DefaultExtra<T> {
    type Extra = (
        CheckSpecVersion<T>,
        CheckTxVersion<T>,
        CheckGenesis<T>,
        CheckEra<T>,
        CheckNonce<T>,
        CheckWeight<T>,
        ChargeTransactionPayment<T>,
    );

    fn new(spec_version: u32, tx_version: u32, nonce: T::Index, genesis_hash: T::Hash) -> Self {
        DefaultExtra {
            spec_version,
            tx_version,
            nonce,
            genesis_hash,
        }
    }

    fn extra(&self) -> Self::Extra {
        (
            CheckSpecVersion(PhantomData, self.spec_version),
            CheckTxVersion(PhantomData, self.tx_version),
            CheckGenesis(PhantomData, self.genesis_hash),
            CheckEra((Era::Immortal, PhantomData), self.genesis_hash),
            CheckNonce(self.nonce),
            CheckWeight(PhantomData),
            ChargeTransactionPayment(<T as Balances>::Balance::default()),
        )
    }
}

impl<T: System + Balances + Debug + Send + Sync> SignedExtension for DefaultExtra<T> {
    const IDENTIFIER: &'static str = "DefaultExtra";
    type AccountId = T::AccountId;
    type Call = ();
    type AdditionalSigned = <<Self as SignedExtra<T>>::Extra as SignedExtension>::AdditionalSigned;
    type Pre = ();

    fn additional_signed(&self) -> Result<Self::AdditionalSigned, TransactionValidityError> {
        self.extra().additional_signed()
    }
}
