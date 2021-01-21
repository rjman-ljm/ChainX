#![cfg_attr(not(feature = "std"), no_std)]

pub mod types {
    use codec::{Decode, Encode};
    use sp_std::vec::Vec;

    pub type BtcAddress = Vec<u8>;

    // #[derive(Encode, Decode, Default, Clone, PartialEq)]
    // #[cfg_attr(feature = "std", derive(Debug))]
    // pub struct IssueRequest<AccountId, BlockNumber, XBTC, PCX> {
    //     pub(crate) vault: AccountId,
    //     pub(crate) opentime: BlockNumber,
    //     pub(crate) requester: AccountId,
    //     pub(crate) btc_address: BtcAddress,
    //     pub(crate) completed: bool,
    //     pub(crate) cancelled: bool,
    //     pub(crate) amount: XBTC,
    //     pub(crate) griefing_collateral: PCX,
    // }
}

#[frame_support::pallet]
#[allow(dead_code)]
pub mod pallet {
    use codec::{Decode, Encode, EncodeLike};
    use frame_support::ensure;
    use frame_support::storage::types::{StorageValue, ValueQuery};
    use frame_support::traits::{Currency, Hooks, ReservableCurrency};
    use frame_support::Twox64Concat;
    use frame_support::{dispatch::DispatchResultWithPostInfo, storage::types::StorageMap};
    use frame_system::{
        ensure_signed,
        pallet_prelude::{BlockNumberFor, OriginFor},
    };
    use sha2::{Digest, Sha256};
    use sp_arithmetic::{
        traits::{CheckedDiv, CheckedMul, UniqueSaturatedInto},
        FixedPointNumber,
    };
    use sp_core::{H256, U256};
    use sp_runtime::{DispatchError, DispatchResult};
    use sp_std::convert::TryInto;
    use sp_std::marker::PhantomData;

    use crate::vault::pallet as vault;

    pub type PCX<T> =
        <<T as vault::Config>::PCX as Currency<<T as frame_system::Config>::AccountId>>::Balance;
    pub type XBTC<T> =
        <<T as Config>::XBTC as Currency<<T as frame_system::Config>::AccountId>>::Balance;
    pub type Inner<T> =
        <<T as Config>::UnsignedFixedPoint as sp_arithmetic::FixedPointNumber>::Inner;

    // pub type IssueRequest<T: Config> = super::types::IssueRequest<
    //     T::AccountId,
    //     T::BlockNumber,
    //     T::XBTC,
    //     <T as vault::Config>::PCX,
    // >;

    pub type UnsignedFixedPointOf<T> = <T as Config>::UnsignedFixedPoint;

    #[pallet::pallet]
    #[pallet::generate_store(pub(crate) trait Store)]
    pub struct Pallet<T>(PhantomData<T>);

    #[pallet::config]
    pub trait Config: frame_system::Config + vault::Config {
        type SecureId;
        type XBTC: ReservableCurrency<Self::AccountId>;
        type UnsignedFixedPoint: FixedPointNumber + Encode + EncodeLike + Decode;
    }

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {}

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// User request issue
        #[pallet::weight(0)]
        pub fn request_issue(
            origin: OriginFor<T>,
            vault_id: T::AccountId,
            amount: XBTC<T>,
            collateral: PCX<T>,
        ) -> DispatchResultWithPostInfo {
            //FIXME(wangyafei): break if bridge in liquidation mode.
            let sender = ensure_signed(origin)?;
            let height = <frame_system::Pallet<T>>::block_number();
            let vault = vault::Pallet::<T>::get_active_vault_by_id(&vault_id)?;
            let required_collateral = Self::get_required_collateral_from_btc_amount(amount)?;
            ensure!(
                collateral >= required_collateral,
                Error::<T>::InsufficientGriefingCollateral
            );
            <T as vault::Config>::PCX::reserve(&sender, collateral)
                .map_err(|_| Error::<T>::InsufficientFunds)?;
            let key = Self::gen_secure_key(sender.clone());
            // Self::insert_issue_request(
            //     key,
            //     IssueRequest {
            //         vault: vault_id.clone(),
            //         opentime: height,
            //         requester: sender,
            //         btc_address: vault.wallet,
            //         amount,
            //         griefing_collateral: collateral,
            //         ..Default::default()
            //     },
            // );
            Ok(().into())
        }
    }

    #[pallet::error]
    pub enum Error<T> {
        /// Collateral in request is less than griefing collateral.
        InsufficientGriefingCollateral,
        /// User does not have enough pcx.
        InsufficientFunds,
        /// Unable to convert value
        TryIntoError,
        ArithmeticOverflow,
        ArithmeticUnderflow,
    }

    #[pallet::storage]
    #[pallet::getter(fn exchange_rate)]
    pub(crate) type ExchangeRateFromBtcToPcx<T: Config> = StorageValue<_, u128, ValueQuery>;

    #[pallet::storage]
    #[pallet::getter(fn issue_griefing_fee)]
    pub(crate) type IssueGriefingFee<T: Config> =
        StorageValue<_, UnsignedFixedPointOf<T>, ValueQuery>;

    #[pallet::storage]
    pub(crate) type Nonce<T: Config> = StorageValue<_, U256, ValueQuery>;

    // #[pallet::storage]
    // pub(crate) type IssueRequests<T: Config> = StorageMap<_, Twox64Concat, H256, IssueRequest<T>>;

    impl<T: Config> Pallet<T> {
        // pub fn insert_issue_request(key: H256, value: IssueRequest<T>) {
        //     <IssueRequests<T>>::insert(&key, value)
        // }
        fn gen_secure_key(id: T::AccountId) -> H256 {
            let nonce = <Nonce<T>>::mutate(|n| {
                *n += U256::one();
                *n
            });
            let mut hasher = Sha256::default();
            hasher.input(id.encode());
            hasher.input(nonce.encode());
            let mut result = [0u8; 32];
            result.copy_from_slice(&hasher.result()[..]);
            H256(result)
        }
        fn into_u128<I: TryInto<u128>>(x: I) -> Result<u128, DispatchError> {
            TryInto::<u128>::try_into(x).map_err(|_| Error::<T>::TryIntoError.into())
        }

        fn btc_to_pcx(amount: XBTC<T>) -> Result<PCX<T>, DispatchError> {
            let raw_amount = Self::into_u128(amount)?;
            let rate = Self::exchange_rate()
                .checked_mul(raw_amount)
                .ok_or(Error::<T>::ArithmeticOverflow)?
                .checked_div(100_000u128)
                .ok_or(Error::<T>::ArithmeticUnderflow)?;
            let result = rate.try_into().map_err(|e| Error::<T>::TryIntoError)?;
            Ok(result)
        }

        fn pcx_to_btc(amount: PCX<T>) -> Result<XBTC<T>, DispatchError> {
            let raw_amount = Self::into_u128(amount)?;
            let rate = raw_amount
                .checked_mul(100_000u128)
                .ok_or(Error::<T>::ArithmeticOverflow)?
                .checked_div(Self::exchange_rate())
                .ok_or(Error::<T>::ArithmeticUnderflow)?;
            let result = rate.try_into().map_err(|e| Error::<T>::TryIntoError)?;
            Ok(result)
        }

        fn pcx_to_inner(x: PCX<T>) -> Result<Inner<T>, DispatchError> {
            let y = TryInto::<u128>::try_into(x).map_err(|_| Error::<T>::TryIntoError)?;
            TryInto::<Inner<T>>::try_into(y).map_err(|_| Error::<T>::TryIntoError.into())
        }

        fn inner_to_pcx(x: Inner<T>) -> Result<PCX<T>, DispatchError> {
            let y = UniqueSaturatedInto::<u128>::unique_saturated_into(x);
            TryInto::<PCX<T>>::try_into(y).map_err(|_| Error::<T>::TryIntoError.into())
        }

        pub fn get_required_collateral_from_btc_amount(
            amount: XBTC<T>,
        ) -> Result<PCX<T>, DispatchError> {
            let pcx_amount = Self::btc_to_pcx(amount)?;
            let percetage = Self::issue_griefing_fee();
            let inner_fee =
                T::UnsignedFixedPoint::checked_from_integer(Self::pcx_to_inner(pcx_amount)?)
                    .ok_or(Error::<T>::ArithmeticOverflow)?
                    .checked_mul(&percetage)
                    .ok_or(Error::<T>::ArithmeticOverflow)?
                    .into_inner()
                    .checked_div(&T::UnsignedFixedPoint::accuracy())
                    .ok_or(Error::<T>::ArithmeticUnderflow)?;
            let pcx_fee = Self::inner_to_pcx(inner_fee)?;
            Ok(pcx_fee)
        }
    }
}