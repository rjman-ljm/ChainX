use std::{fmt::Debug, marker::PhantomData};

use codec::{Decode, Encode};
use sp_runtime::{
    generic::Era, traits::SignedExtension, transaction_validity::TransactionValidityError,
};

use crate::runtime::frame::{balances::Balances, system::System};

/// Ensure the runtime version registered in the transaction is the same as at present.
#[derive(Encode, Decode, Clone, Eq, PartialEq, Debug)]
pub struct CheckSpecVersion<T: System + Send + Sync>(
    pub PhantomData<T>,
    /// Local version to be used for `AdditionalSigned`
    #[codec(skip)]
    pub u32,
);

impl<T: System + Debug + Send + Sync> SignedExtension for CheckSpecVersion<T> {
    const IDENTIFIER: &'static str = "CheckSpecVersion";

    type AccountId = T::AccountId;
    type Call = ();
    type AdditionalSigned = u32;
    type Pre = ();

    fn additional_signed(&self) -> Result<Self::AdditionalSigned, TransactionValidityError> {
        // Ok(<Module<T>>::runtime_version().spec_version)
        Ok(self.1)
    }
}

/// Ensure the transaction version registered in the transaction is the same as at present.
#[derive(Encode, Decode, Clone, Eq, PartialEq, Debug)]
pub struct CheckTxVersion<T: System + Send + Sync>(
    pub PhantomData<T>,
    /// Local version to be used for `AdditionalSigned`
    #[codec(skip)]
    pub u32,
);

impl<T: System + Debug + Send + Sync> SignedExtension for CheckTxVersion<T> {
    const IDENTIFIER: &'static str = "CheckTxVersion";

    type AccountId = T::AccountId;
    type Call = ();
    type AdditionalSigned = u32;
    type Pre = ();

    fn additional_signed(&self) -> Result<Self::AdditionalSigned, TransactionValidityError> {
        // Ok(<Module<T>>::runtime_version().transaction_version)
        Ok(self.1)
    }
}

/// Genesis hash check to provide replay protection between different networks.
#[derive(Encode, Decode, Clone, Eq, PartialEq, Debug)]
pub struct CheckGenesis<T: System + Send + Sync>(
    pub PhantomData<T>,
    /// Local genesis hash to be used for `AdditionalSigned`
    #[codec(skip)]
    pub T::Hash,
);

impl<T: System + Debug + Send + Sync> SignedExtension for CheckGenesis<T> {
    const IDENTIFIER: &'static str = "CheckGenesis";

    type AccountId = T::AccountId;
    type Call = ();
    type AdditionalSigned = T::Hash;
    type Pre = ();

    fn additional_signed(&self) -> Result<Self::AdditionalSigned, TransactionValidityError> {
        // Ok(<Module<T>>::block_hash(T::BlockNumber::zero()))
        Ok(self.1)
    }
}

// Backward compatible re-export.
pub use self::CheckMortality as CheckEra;

/// Check for transaction mortality.
///
/// # Note
///
/// This is modified from the substrate version to allow passing in of the genesis hash, which is
/// returned via `additional_signed()`. It assumes therefore `Era::Immortal` (The transaction is
/// valid forever)
#[derive(Encode, Decode, Clone, Eq, PartialEq, Debug)]
pub struct CheckMortality<T: System + Send + Sync>(
    pub (Era, PhantomData<T>),
    /// Local genesis hash to be used for `AdditionalSigned`
    #[codec(skip)]
    pub T::Hash,
);

impl<T: System + Debug + Send + Sync> SignedExtension for CheckMortality<T> {
    // TODO [#6483] rename to CheckMortality
    const IDENTIFIER: &'static str = "CheckEra";

    type AccountId = T::AccountId;
    type Call = ();
    type AdditionalSigned = T::Hash;
    type Pre = ();

    fn additional_signed(&self) -> Result<Self::AdditionalSigned, TransactionValidityError> {
        /*
        let current_u64 = <Module<T>>::block_number().saturated_into::<u64>();
        let n = self.0.birth(current_u64).saturated_into::<T::BlockNumber>();
        if !<BlockHash<T>>::contains_key(n) {
            Err(InvalidTransaction::AncientBirthBlock.into())
        } else {
            Ok(<Module<T>>::block_hash(n))
        }
        */
        Ok(self.1)
    }
}

/// Nonce check and increment to give replay protection for transactions.
#[derive(Encode, Decode, Clone, Eq, PartialEq, Debug)]
pub struct CheckNonce<T: System + Send + Sync>(#[codec(compact)] pub T::Index);

impl<T: System + Debug + Send + Sync> SignedExtension for CheckNonce<T> {
    const IDENTIFIER: &'static str = "CheckNonce";

    type AccountId = T::AccountId;
    type Call = ();
    type AdditionalSigned = ();
    type Pre = ();

    fn additional_signed(&self) -> Result<Self::AdditionalSigned, TransactionValidityError> {
        Ok(())
    }
}

/// Block resource (weight) limit check.
#[derive(Encode, Decode, Clone, Eq, PartialEq, Debug)]
pub struct CheckWeight<T: System + Send + Sync>(pub PhantomData<T>);

impl<T: System + Debug + Send + Sync> SignedExtension for CheckWeight<T> {
    const IDENTIFIER: &'static str = "CheckWeight";

    type AccountId = T::AccountId;
    type Call = ();
    type AdditionalSigned = ();
    type Pre = ();

    fn additional_signed(&self) -> Result<Self::AdditionalSigned, TransactionValidityError> {
        Ok(())
    }
}

/// Require the transactor pay for themselves and maybe include a tip to gain additional priority
/// in the queue.
#[derive(Encode, Decode, Clone, Eq, PartialEq, Debug)]
pub struct ChargeTransactionPayment<T: Balances + Send + Sync>(#[codec(compact)] pub T::Balance);

impl<T: Balances + Debug + Send + Sync> SignedExtension for ChargeTransactionPayment<T> {
    const IDENTIFIER: &'static str = "ChargeTransactionPayment";

    type AccountId = T::AccountId;
    type Call = ();
    type AdditionalSigned = ();
    type Pre = ();

    fn additional_signed(&self) -> Result<Self::AdditionalSigned, TransactionValidityError> {
        Ok(())
    }
}
