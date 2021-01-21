// Copyright 2019-2020 ChainX Project Authors. Licensed under GPL-3.0.

//!

// Ensure we're `no_std` when compiling for Wasm.
#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

#[cfg(not(feature = "std"))]
use alloc::{format, string::String};

use frame_support::{
    debug, decl_error, decl_event, decl_module, decl_storage, dispatch::Parameter, traits::Get,
    StorageValue,
};

use frame_system::{
    ensure_none,
    offchain::{CreateSignedTransaction, SendTransactionTypes, SubmitTransaction},
};
use sp_application_crypto::RuntimeAppPublic;
use sp_core::crypto::KeyTypeId;
use sp_runtime::{
    offchain::{http, Duration},
    transaction_validity::{
        InvalidTransaction, TransactionPriority, TransactionSource, TransactionValidity,
        ValidTransaction,
    },
};

use sp_std::{
    collections::btree_set::BTreeSet,
    convert::{TryFrom, TryInto},
    marker::PhantomData,
    str, vec,
    vec::Vec,
};

use light_bitcoin::merkle::PartialMerkleTree;
use light_bitcoin::{
    chain::{Block as BtcBlock, BlockHeader as BtcHeader, Transaction as BtcTransaction},
    keys::{Address as BtcAddress, Network as BtcNetwork},
    primitives::{hash_rev, H256 as BtcHash},
    serialization::{deserialize, serialize, Reader},
};
use xp_gateway_bitcoin::{BtcDepositInfo, BtcTxMetaType, BtcTxTypeDetector, OpReturnExtractor};
use xp_gateway_common::{from_ss58_check, AccountExtractor};
use xpallet_assets::Chain;
use xpallet_gateway_bitcoin::{
    types::{BtcDepositCache, BtcRelayedTxInfo, VoteResult},
    Module as XGatewayBitcoin,
};
use xpallet_gateway_common::{trustees::bitcoin::BtcTrusteeAddrInfo, Module as XGatewayCommon};
/// Defines application identifier for crypto keys of this module.
///
/// Every module that deals with signatures needs to declare its unique identifier for
/// its crypto keys.
/// When offchain worker is signing transactions it's going to request keys of type
/// `KeyTypeId` from the keystore and use the ones it finds to sign the transaction.
/// The keys can be inserted manually via RPC (see `author_insertKey`).
pub const BTC_RELAY: KeyTypeId = KeyTypeId(*b"btcr");

/// Based on the above `KeyTypeId` we need to generate a pallet-specific crypto type wrappers.
/// We can use from supported crypto kinds (`sr25519`, `ed25519` and `ecdsa`) and augment
/// the types with this pallet-specific identifier.
mod app {
    pub use super::BTC_RELAY;
    use sp_runtime::app_crypto::{app_crypto, sr25519};
    app_crypto!(sr25519, BTC_RELAY);
}

sp_application_crypto::with_pair! {
    /// An bitcoin offchain keypair using sr25519 as its crypto.
    pub type AuthorityPair = app::Pair;
}

/// An bitcoin offchain identifier using sr25519 as its crypto.
pub type AuthorityId = app::Public;

/// An bitcoin offchain signature using sr25519 as its crypto.
pub type AuthoritySignature = app::Signature;

/// This pallet's configuration trait
pub trait Trait:
    SendTransactionTypes<Call<Self>>
    + CreateSignedTransaction<Call<Self>>
    + xpallet_gateway_bitcoin::Trait
    + xpallet_gateway_common::Trait
{
    /// The overarching event type.
    type Event: From<Event<Self>> + Into<<Self as frame_system::Trait>::Event>;
    /// The overarching dispatch call type.
    type Call: From<Call<Self>>;
    /// A configuration for base priority of unsigned transactions.
    /// This is exposed so that it can be tuned for particular runtime, when
    /// multiple pallets send unsigned transactions.
    type UnsignedPriority: Get<TransactionPriority>;

    /// The identifier type for an offchain worker.
    type AuthorityId: Parameter + Default + RuntimeAppPublic + Ord; // AppCrypto<Self::Public, Self::Signature>;
}

decl_event!(
    /// Events generated by the module.
    pub enum Event<T>
    where
        <T as frame_system::Trait>::AccountId
    {
        /// A Bitcoin block generated. [btc_block_height, btc_block_hash]
        NewBtcBlock(u32, BtcHash),
        /// A Bitcoin transaction. [btc_tx_hash]
        NewBtcTransaction(BtcHash),
        _PhantomData(PhantomData::<AccountId>),
    }
);

decl_error! {
    /// Error for the the module
    pub enum Error for Module<T: Trait> {
        /// Offchain HTTP I/O error.
        HttpIoError,
        /// Offchain HTTP deadline reached.
        HttpDeadlineReached,
        /// Offchain HTTP unknown error.
        HttpUnknown,
        /// Offchain HTTP body is not UTF-8.
        HttpBodyNotUTF8,
        /// Bitcoin serialization/deserialization error.
        BtcSserializationError,
        /// Btc send raw transaction rpc error.
        BtcSendRawTxError,
    }
}

impl<T: Trait> From<sp_core::offchain::HttpError> for Error<T> {
    fn from(err: sp_core::offchain::HttpError) -> Self {
        match err {
            sp_core::offchain::HttpError::DeadlineReached => Error::HttpDeadlineReached,
            sp_core::offchain::HttpError::IoError => Error::HttpIoError,
            sp_core::offchain::HttpError::Invalid => Error::HttpUnknown,
        }
    }
}

impl<T: Trait> From<sp_runtime::offchain::http::Error> for Error<T> {
    fn from(err: sp_runtime::offchain::http::Error) -> Self {
        match err {
            sp_runtime::offchain::http::Error::DeadlineReached => Error::HttpDeadlineReached,
            sp_runtime::offchain::http::Error::IoError => Error::HttpIoError,
            sp_runtime::offchain::http::Error::Unknown => Error::HttpUnknown,
        }
    }
}

decl_storage! {
    trait Store for Module<T: Trait> as XGatewayBitcoinOffchain {
        Keys get(fn keys): Vec<T::AuthorityId>;
    }
    add_extra_genesis {
        config(keys): Vec<T::AuthorityId>;
        build(|config| Module::<T>::initialize_keys(&config.keys))
    }
}

decl_module! {
    /// A public part of the pallet.
    pub struct Module<T: Trait> for enum Call where origin: T::Origin {
        fn deposit_event() = default;

        fn offchain_worker(block_number: T::BlockNumber) {
            // Consider setting the frequency of requesting btc data based on the `block_number`
            debug::info!("ChainX Bitcoin Offchain Worker, ChainX Block #{:?}", block_number);
            let number: u64 = block_number.try_into().unwrap_or(0) as u64;
            //let network = XGatewayBitcoin::<T>::network_id();
            let network = BtcNetwork::Mainnet;
            if number == 2 {
                Self::filter_transactions_and_push(network);
            }
            // if number % 5 == 0{
            //     // First, get withdrawal proposal from chain and broadcast to btc network.
            //     Self::withdrawal_proposal_broadcast(network).unwrap();
            //     // Second, filter transactions from confirmed block and push transactions to chain.
            //     Self::filter_transactions_and_push(network);
            //     // Third, get new block from btc network and push block header to chain
            //     Self::get_new_header_and_push(network);
            // }
        }

        #[weight = 0]
        fn push_header(origin, header: BtcHeader) {
            ensure_none(origin)?;
            debug::info!("Push Header From OCW");
            XGatewayBitcoin::<T>::apply_push_header(header).unwrap();
        }

        #[weight = 0]
        fn push_transaction(origin, tx: BtcTransaction, relayed_info: BtcRelayedTxInfo, prev_tx: Option<BtcTransaction>) {
            ensure_none(origin)?;
            debug::info!("Push Transaction From OCW");
            let relay_tx = relayed_info.into_relayed_tx(tx);
            XGatewayBitcoin::<T>::apply_push_transaction(relay_tx, prev_tx).unwrap();
        }

    }
}

impl<T: Trait> Module<T> {
    fn initialize_keys(keys: &[T::AuthorityId]) {
        if !keys.is_empty() {
            assert!(Keys::<T>::get().is_empty(), "Keys are already initialized!");
            Keys::<T>::put(keys);
        }
    }
}

/// Most of the functions are moved outside of the `decl_module!` macro.
///
/// This greatly helps with error messages, as the ones inside the macro
/// can sometimes be hard to debug.
impl<T: Trait> Module<T> {
    // get trustee pair
    fn get_trustee_pair(session_number: u32) -> Result<Option<(BtcAddress, BtcAddress)>, Error<T>> {
        if let Some(trustee_session_info) =
            XGatewayCommon::<T>::trustee_session_info_of(Chain::Bitcoin, session_number)
        {
            let hot_addr: Vec<u8> = trustee_session_info.0.hot_address;
            let hot_addr = BtcTrusteeAddrInfo::try_from(hot_addr).unwrap();
            let hot_addr = String::from_utf8(hot_addr.addr)
                .unwrap()
                .parse::<BtcAddress>()
                .unwrap();
            let cold_addr: Vec<u8> = trustee_session_info.0.cold_address;
            let cold_addr = BtcTrusteeAddrInfo::try_from(cold_addr).unwrap();
            let cold_addr = String::from_utf8(cold_addr.addr)
                .unwrap()
                .parse::<BtcAddress>()
                .unwrap();
            debug::info!("[ChainX|btc_trustee_pair] ChainX X-BTC Trustee Session Info (session number = {:?}):[Hot Address: {}, Cold Address: {}]",
                            session_number,
                            hot_addr,
                            cold_addr,);
            Ok(Some((hot_addr, cold_addr)))
        } else {
            Ok(None)
        }
    }
    // get new header from btc network and push header to chain
    fn get_new_header_and_push(network: BtcNetwork) {
        let best_index = XGatewayBitcoin::<T>::best_index().height;
        let next_height = best_index + 1;
        let btc_block_hash = match Self::fetch_block_hash(next_height, network) {
            Ok(Some(hash)) => {
                debug::info!("₿ Block #{} hash: {}", next_height, hash);
                hash
            }
            Ok(None) => {
                debug::warn!("₿ Block #{} has not been generated yet", next_height);
                return;
            }
            Err(err) => {
                debug::warn!("₿ {:?}", err);
                return;
            }
        };

        let btc_block = match Self::fetch_block(&btc_block_hash[..], network) {
            Ok(block) => {
                debug::info!("₿ Block {}", hash_rev(block.hash()));
                block
            }
            Err(err) => {
                debug::warn!("₿ {:?}", err);
                return;
            }
        };
        let btc_header = btc_block.header;
        if XGatewayBitcoin::<T>::block_hash_for(best_index)
            .contains(&btc_header.previous_header_hash)
        {
            let call = Call::push_header(btc_block.header);
            debug::info!(
                "₿ Submitting unsigned transaction for pushing header: {:?}",
                call
            );
            if let Err(e) =
                SubmitTransaction::<T, Call<T>>::submit_unsigned_transaction(call.into())
            {
                debug::error!(
                    "Failed to submit unsigned transaction for pushing header: {:?}",
                    e
                );
            }
        }
    }
    // filter transactions in confirmed block and push transactions to chain
    fn filter_transactions_and_push(network: BtcNetwork) {
        //if let Some(confirmed_index) = XGatewayBitcoin::<T>::confirmed_index() {
        // let confirm_height = confirmed_index.height;
        let confirm_height = 666814;
        let confirm_hash = match Self::fetch_block_hash(confirm_height, network) {
            Ok(Some(hash)) => {
                debug::info!("₿ Confirmed Block #{} hash: {}", confirm_height, hash);
                hash
            }
            Ok(None) => {
                debug::warn!(
                    "₿ Confirmed Block #{} has not been generated yet",
                    confirm_height
                );
                return;
            }
            Err(err) => {
                debug::warn!("₿ Confirmed {:?}", err);
                return;
            }
        };

        let btc_confirmed_block = match Self::fetch_block(&confirm_hash[..], network) {
            Ok(block) => {
                debug::info!("₿ Confirmed Block {}", hash_rev(block.hash()));
                block
            }
            Err(err) => {
                debug::warn!("₿ Confirmed {:?}", err);
                return;
            }
        };

        Self::push_xbtc_transaction(&btc_confirmed_block, network);
        //}
    }
    // Submit XBTC deposit/withdraw transaction to the ChainX
    fn push_xbtc_transaction(confirmed_block: &BtcBlock, network: BtcNetwork) {
        let mut needed = Vec::new();
        let mut tx_hashes = Vec::with_capacity(confirmed_block.transactions.len());
        let mut tx_matches = Vec::with_capacity(confirmed_block.transactions.len());

        // get trustee info
        let trustee_session_info_len =
            XGatewayCommon::<T>::trustee_session_info_len(Chain::Bitcoin);
        debug::info!(
            "[OCW] trustee_session_info_len: {}",
            trustee_session_info_len
        );
        let current_trustee_session_number = trustee_session_info_len
            .checked_sub(1)
            .unwrap_or(u32::max_value());
        let previous_trustee_session_number = trustee_session_info_len
            .checked_sub(2)
            .unwrap_or(u32::max_value());
        let current_trustee_pair = Self::get_trustee_pair(current_trustee_session_number)
            .unwrap()
            .unwrap();
        let previous_trustee_pair = match Self::get_trustee_pair(previous_trustee_session_number) {
            Ok(Some((hot, cold))) => (hot, cold),
            Ok(None) => current_trustee_pair,
            Err(_e) => current_trustee_pair,
        };
        let btc_min_deposit = XGatewayBitcoin::<T>::btc_min_deposit();
        // construct BtcTxTypeDetector
        let btc_tx_detector =
            BtcTxTypeDetector::new(network, btc_min_deposit, current_trustee_pair, None);

        let tx = Self::fetch_transaction(
            "9d9cad00589d96a65747da6b6a1bf1b99a1e109684934013151bafaeb3b0c994",
            network,
        )
        .unwrap();
        //for tx in &confirmed_block.transactions {
        // Prepare for constructing partial merkle tree
        tx_hashes.push(tx.hash());
        if tx.is_coinbase() {
            tx_matches.push(false);
            //continue;
        }
        let outpoint = tx.inputs[0].previous_output;
        let prev_tx_hash = hex::encode(hash_rev(outpoint.txid));
        let prev_tx = Self::fetch_transaction(&prev_tx_hash[..], network)
            .expect("Fail to get prev tx from btc network");

        // Detect X-BTC transaction type
        // Withdrawal: must have a previous transaction
        // Deposit: don't require previous transaction generally,
        //          but in special cases, a previous transaction needs to be submitted.
        match btc_tx_detector.detect_transaction_type(
            &tx,
            Some(&prev_tx),
            OpReturnExtractor::extract_account,
        ) {
            BtcTxMetaType::Withdrawal => {
                debug::info!(
                    "X-BTC Withdrawal (PrevTx: {:?}, Tx: {:?})",
                    hash_rev(prev_tx.hash()),
                    hash_rev(tx.hash())
                );
                tx_matches.push(true);
                needed.push((tx.clone(), Some(prev_tx)));
            }
            BtcTxMetaType::Deposit(BtcDepositInfo {
                deposit_value,
                op_return,
                input_addr,
            }) => {
                debug::info!(
                    "X-BTC Deposit [{}] (Tx: {:?})",
                    deposit_value,
                    hash_rev(tx.hash()),
                );
                debug::info!("X-BTC Deposit OpReturn:[{:?}]", op_return);
                tx_matches.push(true);
                match (input_addr, op_return) {
                    (_, Some((account, _))) => {
                        if Self::pending_deposits(account).is_empty() {
                            needed.push((tx.clone(), None));
                        } else {
                            needed.push((tx.clone(), Some(prev_tx)));
                        }
                    }
                    (Some(_), None) => needed.push((tx.clone(), Some(prev_tx))),
                    (None, None) => {
                        debug::warn!(
                                "[Service|push_xbtc_transaction] parsing prev_tx or op_return error, tx {:?}",
                                hash_rev(tx.hash())
                            );
                        needed.push((tx.clone(), Some(prev_tx)));
                    }
                }
            }
            BtcTxMetaType::HotAndCold
            | BtcTxMetaType::TrusteeTransition
            | BtcTxMetaType::Irrelevance => tx_matches.push(false),
        }
        // }

        if !needed.is_empty() {
            debug::info!(
                "[Service|push_xbtc_transaction] Generate partial merkle tree from the Confirmed Block {:?}",
                hash_rev(confirmed_block.hash())
            );

            // Construct partial merkle tree
            // We can never have zero txs in a merkle block, we always need the coinbase tx.
            let merkle_proof = PartialMerkleTree::from_txids(&tx_hashes, &tx_matches);

            // Push xbtc relay (withdraw/deposit) transaction
            for (tx, prev_tx) in needed {
                let relayed_info = BtcRelayedTxInfo {
                    block_hash: confirmed_block.hash(),
                    merkle_proof: merkle_proof.clone(),
                };

                let call = Call::push_transaction(tx, relayed_info, prev_tx);
                if let Err(e) =
                    SubmitTransaction::<T, Call<T>>::submit_unsigned_transaction(call.into())
                {
                    debug::error!(
                        "Failed to submit unsigned transaction for pushing transaction: {:?}",
                        e
                    );
                }
            }
        } else {
            debug::info!(
                "[Service|push_xbtc_transaction] No X-BTC Deposit/Withdraw Transactions in th Confirmed Block {:?}",
                hash_rev(confirmed_block.hash())
            );
        }
    }
    // help use AccountId
    fn pending_deposits<P: AsRef<[u8]>>(btc_address: P) -> Vec<BtcDepositCache> {
        let btc_address = btc_address.as_ref();
        let deposit_cache: Vec<BtcDepositCache> =
            XGatewayBitcoin::<T>::pending_deposits(btc_address);
        deposit_cache
    }
    // get withdrawal proposal from chain and broadcast raw transaction
    fn withdrawal_proposal_broadcast(network: BtcNetwork) -> Result<Option<String>, ()> {
        if let Some(withdrawal_proposal) = XGatewayBitcoin::<T>::withdrawal_proposal() {
            if withdrawal_proposal.sig_state == VoteResult::Finish {
                let tx = serialize(&withdrawal_proposal.tx).take();
                let hex_tx = hex::encode(&tx);
                debug::info!("send_raw_transaction| Btc Tx Hex: {}", hex_tx);
                match Self::send_raw_transaction(hex_tx, network) {
                    Ok(hash) => {
                        debug::info!("send_raw_transaction| Transaction Hash: {:?}", hash);
                        return Ok(Some(hash));
                    }
                    Err(err) => {
                        debug::warn!("send_raw_transaction| Error {:?}", err);
                    }
                }
            }
        }
        Ok(None)
    }

    fn get<U: AsRef<str>>(url: U) -> Result<Vec<u8>, Error<T>> {
        // We want to keep the offchain worker execution time reasonable, so we set a hard-coded
        // deadline to 2s to complete the external call.
        // You can also wait indefinitely for the response, however you may still get a timeout
        // coming from the host machine.
        let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(2_000));

        // Initiate an external HTTP GET request.
        // This is using high-level wrappers from `sp_runtime`, for the low-level calls that
        // you can find in `sp_io`. The API is trying to be similar to `reqwest`, but
        // since we are running in a custom WASM execution environment we can't simply
        // import the library here.
        // We set the deadline for sending of the request, note that awaiting response can
        // have a separate deadline. Next we send the request, before that it's also possible
        // to alter request headers or stream body content in case of non-GET requests.
        let pending = http::Request::get(url.as_ref())
            .deadline(deadline)
            .send()
            .map_err(|err| Error::<T>::from(err))?;

        // The request is already being processed by the host, we are free to do anything
        // else in the worker (we can send multiple concurrent requests too).
        // At some point however we probably want to check the response though,
        // so we can block current thread and wait for it to finish.
        // Note that since the request is being driven by the host, we don't have to wait
        // for the request to have it complete, we will just not read the response.
        let response = pending
            .try_wait(deadline)
            .map_err(|_| Error::<T>::HttpDeadlineReached)??;

        // Let's check the status code before we proceed to reading the response.
        if response.code != 200 {
            debug::warn!("Unexpected status code: {}", response.code);
            return Err(Error::<T>::HttpUnknown);
        }

        // Next we want to fully read the response body and collect it to a vector of bytes.
        // Note that the return object allows you to read the body in chunks as well
        // with a way to control the deadline.
        let resp_body = response.body().collect::<Vec<u8>>();
        Ok(resp_body)
    }

    fn post<B, I>(url: &str, req_body: B) -> Result<Vec<u8>, Error<T>>
    where
        B: Default + IntoIterator<Item = I>,
        I: AsRef<[u8]>,
    {
        // We want to keep the offchain worker execution time reasonable, so we set a hard-coded
        // deadline to 2s to complete the external call.
        // You can also wait indefinitely for the response, however you may still get a timeout
        // coming from the host machine.
        let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(2_000));

        // Initiate an external HTTP POST request.
        // This is using high-level wrappers from `sp_runtime`, for the low-level calls that
        // you can find in `sp_io`. The API is trying to be similar to `reqwest`, but
        // since we are running in a custom WASM execution environment we can't simply
        // import the library here.
        // We set the deadline for sending of the request, note that awaiting response can
        // have a separate deadline. Next we send the request, before that it's also possible
        // to alter request headers or stream body content in case of non-GET requests.
        let pending = http::Request::post(url, req_body)
            .deadline(deadline)
            .send()
            .map_err(|err| Error::<T>::from(err))?;

        // The request is already being processed by the host, we are free to do anything
        // else in the worker (we can send multiple concurrent requests too).
        // At some point however we probably want to check the response though,
        // so we can block current thread and wait for it to finish.
        // Note that since the request is being driven by the host, we don't have to wait
        // for the request to have it complete, we will just not read the response.
        let response = pending
            .try_wait(deadline)
            .map_err(|_| Error::<T>::HttpDeadlineReached)??;

        // Let's check the status code before we proceed to reading the response.
        if response.code != 200 {
            debug::warn!("Unexpected status code: {}", response.code);
            return Err(Error::<T>::HttpUnknown);
        }

        // Next we want to fully read the response body and collect it to a vector of bytes.
        // Note that the return object allows you to read the body in chunks as well
        // with a way to control the deadline.
        let resp_body = response.body().collect::<Vec<u8>>();
        Ok(resp_body)
    }

    fn fetch_block_hash(height: u32, network: BtcNetwork) -> Result<Option<String>, Error<T>> {
        let url = match network {
            BtcNetwork::Mainnet => format!("https://blockstream.info/api/block-height/{}", height),
            BtcNetwork::Testnet => format!(
                "https://blockstream.info/testnet/api/block-height/{}",
                height
            ),
        };
        debug::info!("[OCW] get url: {}", url);
        let resp_body = Self::get(url)?;
        let resp_body = str::from_utf8(&resp_body).map_err(|_| {
            debug::warn!("No UTF8 body");
            Error::<T>::HttpBodyNotUTF8
        })?;

        const RESP_BLOCK_NOT_FOUND: &str = "Block not found";
        if resp_body == RESP_BLOCK_NOT_FOUND {
            debug::info!("₿ Block #{} not found", height);
            Ok(None)
        } else {
            let hash: String = resp_body.into();
            debug::info!("₿ Block #{} hash: {:?}", height, hash);
            Ok(Some(hash))
        }
    }

    fn fetch_block(hash: &str, network: BtcNetwork) -> Result<BtcBlock, Error<T>> {
        let url = match network {
            BtcNetwork::Mainnet => format!("https://blockstream.info/api/block/{}/raw", hash),
            BtcNetwork::Testnet => {
                format!("https://blockstream.info/testnet/api/block/{}/raw", hash)
            }
        };
        let body = Self::get(url)?;
        let block = deserialize::<_, BtcBlock>(Reader::new(&body))
            .map_err(|_| Error::<T>::BtcSserializationError)?;

        debug::info!("₿ Block {}", hash_rev(block.hash()));
        Ok(block)
    }

    fn fetch_transaction(hash: &str, network: BtcNetwork) -> Result<BtcTransaction, Error<T>> {
        let url = match network {
            BtcNetwork::Mainnet => format!("https://blockstream.info/api/tx/{}/raw", hash),
            BtcNetwork::Testnet => format!("https://blockstream.info/testnet/api/tx/{}/raw", hash),
        };
        let body = Self::get(url)?;
        let transaction = deserialize::<_, BtcTransaction>(Reader::new(&body))
            .map_err(|_| Error::<T>::BtcSserializationError)?;
        debug::info!("₿ Transaction {}", hash_rev(transaction.hash()));
        Ok(transaction)
    }

    fn send_raw_transaction<TX: AsRef<[u8]>>(
        hex_tx: TX,
        network: BtcNetwork,
    ) -> Result<String, Error<T>> {
        let url = match network {
            BtcNetwork::Mainnet => "https://blockstream.info/api/tx",
            BtcNetwork::Testnet => "https://blockstream.info/testnet/api/tx",
        };
        let resp_body = Self::post(url, vec![hex_tx.as_ref()])?;
        let resp_body = str::from_utf8(&resp_body).map_err(|_| {
            debug::warn!("No UTF8 body");
            Error::<T>::HttpBodyNotUTF8
        })?;

        if resp_body.len() == 2 * BtcHash::len_bytes() {
            let hash: String = resp_body.into();
            debug::info!(
                "₿ Send Transaction successfully, Hash: {}, HexTx: {}",
                hash,
                hex::encode(hex_tx.as_ref())
            );
            Ok(hash)
        } else if resp_body.starts_with(SEND_RAW_TX_ERR_PREFIX) {
            if let Some(err) = Self::parse_send_raw_tx_error(resp_body) {
                debug::info!(
                    "₿ Send Transaction error: (code: {}, msg: {}), HexTx: {}",
                    err.code,
                    err.message,
                    hex::encode(hex_tx.as_ref())
                );
            } else {
                debug::info!(
                    "₿ Send Transaction unknown error, HexTx: {}",
                    hex::encode(hex_tx.as_ref())
                );
            }
            Err(Error::<T>::BtcSendRawTxError)
        } else {
            debug::info!(
                "₿ Send Transaction unknown error, HexTx: {}",
                hex::encode(hex_tx.as_ref())
            );
            Err(Error::<T>::BtcSendRawTxError)
        }
    }

    fn parse_send_raw_tx_error(resp_body: &str) -> Option<SendRawTxError> {
        use lite_json::JsonValue;
        let rest_resp = resp_body.trim_start_matches(SEND_RAW_TX_ERR_PREFIX);
        let value = lite_json::parse_json(rest_resp).ok();
        value.and_then(|v| match v {
            JsonValue::Object(obj) => {
                let code = obj
                    .iter()
                    .find(|(k, _)| k == &['c', 'o', 'd', 'e'])
                    .map(|(_, code)| code);
                let message = obj
                    .iter()
                    .find(|(k, _)| k == &['m', 'e', 's', 's', 'a', 'g', 'e'])
                    .map(|(_, msg)| msg);
                match (code, message) {
                    (Some(JsonValue::Number(code)), Some(JsonValue::String(msg))) => {
                        Some(SendRawTxError {
                            code: code.integer,
                            message: msg.iter().collect(),
                        })
                    }
                    _ => None,
                }
            }
            _ => None,
        })
    }
}

const SEND_RAW_TX_ERR_PREFIX: &str = "send raw transaction RPC error: ";
struct SendRawTxError {
    code: i64,
    message: String,
}

impl<T: Trait> sp_runtime::BoundToRuntimeAppPublic for Module<T> {
    type Public = T::AuthorityId;
}

impl<T: Trait> pallet_session::OneSessionHandler<T::AccountId> for Module<T> {
    type Key = T::AuthorityId;

    fn on_genesis_session<'a, I: 'a>(validators: I)
    where
        I: Iterator<Item = (&'a T::AccountId, Self::Key)>,
    {
        let keys = validators.map(|x| x.1).collect::<Vec<_>>();
        Self::initialize_keys(&keys);
    }

    fn on_new_session<'a, I: 'a>(changed: bool, validators: I, queued_validators: I)
    where
        I: Iterator<Item = (&'a T::AccountId, Self::Key)>,
    {
        if changed {
            let keys = validators
                .chain(queued_validators)
                .map(|x| x.1)
                .collect::<BTreeSet<_>>();
            Keys::<T>::put(keys.into_iter().collect::<Vec<_>>());
        }
    }

    fn on_disabled(_validator_index: usize) {}
}

impl<T: Trait> frame_support::unsigned::ValidateUnsigned for Module<T> {
    type Call = Call<T>;

    fn validate_unsigned(_source: TransactionSource, call: &Self::Call) -> TransactionValidity {
        match call {
            Call::push_header(header) => {
                ValidTransaction::with_tag_prefix("XGatewayBitcoinOffchain")
                .priority(T::UnsignedPriority::get())
                .and_provides(header) // TODO: a tag is required, otherwise the transactions will not be pruned.
                // .and_provides((current_session, authority_id)) provide a tag?
                .longevity(1u64) // FIXME a proper longevity
                .propagate(true)
                .build()
            }
            Call::push_transaction(tx, _relayed_info, _prev_tx) => {
                ValidTransaction::with_tag_prefix("XGatewayBitcoinOffchain")
                .priority(T::UnsignedPriority::get())
                .and_provides(tx) // TODO: a tag is required, otherwise the transactions will not be pruned.
                // .and_provides((current_session, authority_id)) provide a tag?
                .longevity(1u64) // FIXME a proper longevity
                .propagate(true)
                .build()
            }
            _ => InvalidTransaction::Call.into(),
        }
    }
}
