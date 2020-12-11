use std::fmt::Debug;

use futures::future::BoxFuture;

use codec::{Codec, Decode, Encode};
use frame_support::Parameter;
use sp_runtime::traits::{AtLeast32BitUnsigned, MaybeSerializeDeserialize, Member};

use crate::{
    client::{Client, ExtrinsicSuccess},
    error::Error,
    events::EventsDecoder,
    runtime::{
        frame::system::{System, SystemEventsDecoder},
        Call, Event, Runtime, Signer,
    },
};

// ============================================================================
// Module
// ============================================================================

/// The subset of the `pallet_balances::Trait`.
pub trait Balances: System {
    /// The balance of an account.
    type Balance: Parameter
        + Member
        + AtLeast32BitUnsigned
        + Codec
        + Default
        + Copy
        + MaybeSerializeDeserialize
        + Debug;
}

pub const MODULE: &str = "Balances";

/// `EventsDecoder` extension trait.
pub trait BalancesEventsDecoder {
    /// Registers this modules types.
    fn with_balances(&mut self);
}

impl<T: Balances> BalancesEventsDecoder for EventsDecoder<T> {
    fn with_balances(&mut self) {
        self.with_system();
        self.register_type_size::<T::Balance>("Balance");
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Encode, Decode)]
pub struct AccountData<Balance> {
    /// Non-reserved part of the balance. There may still be restrictions on this, but it is the
    /// total pool what may in principle be transferred, reserved and used for tipping.
    ///
    /// This is the only balance that matters in terms of most operations on tokens. It
    /// alone is used to determine the balance when in the contract execution environment.
    pub free: Balance,
    /// Balance which is reserved and may not be used at all.
    ///
    /// This can still get slashed, but gets slashed last of all.
    ///
    /// This balance is a 'reserve' balance that other subsystems use in order to set aside tokens
    /// that are still 'owned' by the account holder, but which are suspendable.
    pub reserved: Balance,
    /// The amount that `free` may not drop below when withdrawing for *anything except transaction
    /// fee payment*.
    pub misc_frozen: Balance,
    /// The amount that `free` may not drop below when withdrawing specifically for transaction
    /// fee payment.
    pub fee_frozen: Balance,
}

// ============================================================================
// Storage
// ============================================================================

// ============================================================================
// Call
// ============================================================================

pub trait BalancesCallExt<T: Runtime + Balances> {
    fn transfer<'a>(
        &'a self,
        signer: &'a (dyn Signer<T> + Send + Sync),
        dest: &'a <T as System>::Address,
        amount: T::Balance,
    ) -> BoxFuture<'a, Result<T::Hash, Error>>;
    fn transfer_and_watch<'a>(
        &'a self,
        signer: &'a (dyn Signer<T> + Send + Sync),
        dest: &'a <T as System>::Address,
        amount: T::Balance,
    ) -> BoxFuture<'a, Result<ExtrinsicSuccess<T>, Error>>;
}

impl<T: Runtime + Balances> BalancesCallExt<T> for Client<T> {
    fn transfer<'a>(
        &'a self,
        signer: &'a (dyn Signer<T> + Send + Sync),
        dest: &'a <T as System>::Address,
        amount: T::Balance,
    ) -> BoxFuture<'a, Result<T::Hash, Error>> {
        Box::pin(self.submit(TransferCall { dest, amount }, signer))
    }

    fn transfer_and_watch<'a>(
        &'a self,
        signer: &'a (dyn Signer<T> + Send + Sync),
        dest: &'a <T as System>::Address,
        amount: T::Balance,
    ) -> BoxFuture<'a, Result<ExtrinsicSuccess<T>, Error>> {
        Box::pin(self.watch(TransferCall { dest, amount }, signer))
    }
}

/// Transfer some liquid free balance to another account.
///
/// `transfer` will set the `FreeBalance` of the sender and receiver.
/// It will decrease the total issuance of the system by the `TransferFee`.
/// If the sender's account is below the existential deposit as a result
/// of the transfer, the account will be reaped.
#[derive(Clone, Debug, PartialEq, Encode)]
pub struct TransferCall<'a, T: Balances> {
    /// Destination of the transfer.
    pub dest: &'a <T as System>::Address,
    /// Amount to transfer.
    #[codec(compact)]
    pub amount: T::Balance,
}

impl<'a, T: Balances> Call<T> for TransferCall<'a, T> {
    const MODULE: &'static str = MODULE;
    const FUNCTION: &'static str = "transfer";

    fn events_decoder(decoder: &mut EventsDecoder<T>) {
        decoder.with_balances();
    }
}

// ============================================================================
// Event
// ============================================================================

/// Event extension trait.
pub trait BalancesEventExt<T: Balances> {
    fn transfer(&self) -> Result<Option<TransferEvent<T>>, codec::Error>;
}

impl<T: Balances> BalancesEventExt<T> for ExtrinsicSuccess<T> {
    fn transfer(&self) -> Result<Option<TransferEvent<T>>, codec::Error> {
        self.find_event()
    }
}

/// Transfer event.
#[derive(Clone, Debug, Eq, PartialEq, Decode)]
pub struct TransferEvent<T: Balances> {
    /// Account balance was transferred from.
    pub from: <T as System>::AccountId,
    /// Account balance was transferred to.
    pub to: <T as System>::AccountId,
    /// Amount of balance that was transferred.
    pub amount: T::Balance,
}

impl<T: Balances> Event<T> for TransferEvent<T> {
    const MODULE: &'static str = MODULE;
    const EVENT: &'static str = "Transfer";
}
