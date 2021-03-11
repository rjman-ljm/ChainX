<<<<<<< HEAD
use sp_std::cmp::{Eq, PartialEq};
use sp_std::default::Default;
use sp_std::vec::Vec;

use codec::{Decode, Encode};

#[cfg(feature = "std")]
use serde::{Deserialize, Serialize};

#[derive(PartialEq, Eq, Clone, Default, Encode, Decode)]
#[cfg_attr(feature = "std", derive(Serialize, Deserialize, Debug))]
#[cfg_attr(feature = "std", serde(rename_all = "camelCase"))]
pub struct RpcVaultInfo<AccountId> {
    pub account: AccountId,
    #[cfg_attr(feature = "std", serde(with = "xp_rpc::serde_text"))]
    pub btc_addr: Vec<u8>,
}
=======
// rpc module
>>>>>>> btc-addr
