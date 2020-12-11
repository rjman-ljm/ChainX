// Copyright 2019-2020 ChainX Project Authors. Licensed under GPL-3.0.

pub mod frame;
pub mod primitives;

use sp_core::{crypto::Pair as TraitPair, sr25519};
use sp_runtime::traits::BlakeTwo256;

pub use subxt::Signer;
use subxt::{
    balances::{AccountData, Balances},
    extrinsic::DefaultExtra,
    system::System,
    PairSigner, Runtime,
};

use self::{
    frame::{
        xgateway_bitcoin::XGatewayBitcoin, xgateway_common::XGatewayCommon,
        xgateway_records::XGatewayRecords, xsystem::XSystem,
    },
    primitives::{
        AccountId, Address, Balance, BlockNumber, Extrinsic, Hash, Header, Index, Signature,
    },
};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ChainXNodeRuntime;

impl Runtime for ChainXNodeRuntime {
    type Signature = Signature;
    type Extra = ChainXExtra<Self>;
}

impl System for ChainXNodeRuntime {
    type Index = Index;
    type BlockNumber = BlockNumber;
    type Hash = Hash;
    type Hashing = BlakeTwo256;
    type AccountId = AccountId;
    type Address = Address;
    type Header = Header;
    type Extrinsic = Extrinsic;
    type AccountData = AccountData<<Self as Balances>::Balance>;
}

impl Balances for ChainXNodeRuntime {
    type Balance = Balance;
}

impl XSystem for ChainXNodeRuntime {}

impl XGatewayCommon for ChainXNodeRuntime {}

impl XGatewayBitcoin for ChainXNodeRuntime {}

impl XGatewayRecords for ChainXNodeRuntime {}

/// ChainX `SignedExtra` for ChainX runtime.
pub type ChainXExtra<T> = DefaultExtra<T>;

/// ChainX `Pair` for ChainX runtime.
pub type ChainXPair = sr25519::Pair;

/// ChainX `PairSigner` for ChainX runtime.
pub type ChainXPairSigner = PairSigner<ChainXNodeRuntime, ChainXPair>;

/// Generate a crypto pair from seed.
pub fn get_pair_from_seed(seed: &str) -> ChainXPair {
    ChainXPair::from_string(&format!("//{}", seed), None).expect("static values are valid; qed")
}

/// Generate an account ID from seed.
#[cfg(test)]
pub(crate) fn get_account_id_from_seed(seed: &str) -> AccountId {
    use sp_runtime::traits::{IdentifyAccount, Verify};
    let pair = get_pair_from_seed(seed);
    <<ChainXNodeRuntime as Runtime>::Signature as Verify>::Signer::from(pair.public())
        .into_account()
}
