#![cfg_attr(not(feature = "std"), no_std)]
#![allow(clippy::too_many_arguments, clippy::unnecessary_mut_passed)]

use codec::Codec;

pub use xpallet_gateway_bitcoin_v2::rpc::RpcVaultInfo;

sp_api::decl_runtime_apis! {
    pub trait XGatewayBitcoinV2Api<AccountId, BlockNumber, Balance>
        where AccountId:Codec, BlockNumber: Codec, Balance: Codec
    {
        fn get_first_matched_vault(xbtc_amount: Balance) -> Option<RpcVaultInfo<AccountId>>;
    }
}
