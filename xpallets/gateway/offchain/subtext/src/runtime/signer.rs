use codec::Encode;
use sp_core::crypto::Pair;
use sp_runtime::traits::{IdentifyAccount, Verify};

use crate::runtime::{Runtime, SignedPayload, Signer, UncheckedExtrinsic};

/// Extrinsic signer using a private key.
#[derive(Clone)]
pub struct PairSigner<T: Runtime, P: Pair> {
    account_id: T::AccountId,
    nonce: Option<T::Index>,
    signer: P,
}

impl<T, P> PairSigner<T, P>
where
    T: Runtime,
    <T::Signature as Verify>::Signer: From<P::Public> + IdentifyAccount<AccountId = T::AccountId>,
    P: Pair,
{
    /// Creates a new `Signer` from a `Pair`.
    pub fn new(signer: P) -> Self {
        let account_id = <T::Signature as Verify>::Signer::from(signer.public()).into_account();
        Self {
            account_id,
            nonce: None,
            signer,
        }
    }

    /// Sets the nonce to a new value.
    pub fn set_nonce(&mut self, nonce: T::Index) {
        self.nonce = Some(nonce);
    }

    /// Returns the signer.
    pub fn signer(&self) -> &P {
        &self.signer
    }
}

impl<T, P> Signer<T> for PairSigner<T, P>
where
    T: Runtime,
    T::AccountId: Into<T::Address>,
    P: Pair,
    P::Signature: Into<T::Signature>,
{
    fn account_id(&self) -> &T::AccountId {
        &self.account_id
    }

    fn nonce(&self) -> Option<T::Index> {
        self.nonce
    }

    fn sign(&self, extrinsic: SignedPayload<T>) -> Result<UncheckedExtrinsic<T>, String> {
        let signature = extrinsic
            .using_encoded(|payload| self.signer.sign(payload))
            .into();
        let (call, extra, _) = extrinsic.deconstruct();
        let address: T::Address = self.account_id.clone().into();
        let extrinsic = UncheckedExtrinsic::<T>::new_signed(call, address, signature, extra);
        Ok(extrinsic)
    }
}
