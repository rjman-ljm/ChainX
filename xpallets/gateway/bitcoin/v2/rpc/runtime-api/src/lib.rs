#![cfg_attr(not(feature = "std"), no_std)]
#![allow(clippy::too_many_arguments, clippy::unnecessary_mut_passed)]

<<<<<<< HEAD
use codec::Codec;

pub use xpallet_gateway_bitcoin_v2::rpc::RpcVaultInfo;
=======
use sp_std::vec::Vec;

use codec::Codec;
>>>>>>> btc-addr

sp_api::decl_runtime_apis! {
    pub trait XGatewayBitcoinV2Api<AccountId, BlockNumber, Balance>
        where AccountId:Codec, BlockNumber: Codec, Balance: Codec
    {
<<<<<<< HEAD
        fn get_first_matched_vault(xbtc_amount: Balance) -> Option<RpcVaultInfo<AccountId>>;
=======
        fn get_first_matched_vault(xbtc_amount: Balance) -> Option<(AccountId, Vec<u8>)>;
>>>>>>> btc-addr
    }
}
