// Copyright 2019-2020 ChainX Project Authors. Licensed under GPL-3.0.

use std::{cmp::Ordering, convert::TryFrom, time::Duration};

use futures::future;
use sp_core::crypto::{Ss58AddressFormat, Ss58Codec};
use subxt::{
    balances::{AccountData, TransferCallExt, TransferEventExt},
    system::{AccountInfo, AccountStoreExt},
    Client, ClientBuilder, ExtrinsicSuccess, Signer,
};
use tokio::time;

use light_bitcoin::{
    chain::{
        BlockHeader as BtcBlockHeader, Transaction as BtcTransaction,
        TransactionOutput as BtcTransactionOutput,
    },
    keys::{Address as BtcAddress, Network as BtcNetwork},
    primitives::{hash_rev, H256},
    script::{Opcode as BtcScriptOpcode, Script as BtcScript, ScriptType as BtcScriptType},
    serialization::serialize,
};

use crate::{
    error::Error,
    runtime::{
        frame::{
            xgateway_bitcoin::{
                BestIndexStoreExt, BlockHashForStoreExt, BtcDepositCache, BtcHeaderIndex,
                BtcHeaderInfo, BtcMinDepositStoreExt, BtcRelayedTxInfo, BtcWithdrawalFeeStoreExt,
                BtcWithdrawalProposal, ConfirmedIndexStoreExt, GenesisInfoStoreExt,
                HeadersStoreExt, NetworkIdStoreExt, PendingDepositsStoreExt,
                WithdrawalProposalStoreExt,
            },
            xgateway_bitcoin::{PushHeaderCallExt, PushTransactionCallExt},
            xgateway_bitcoin::{PushHeaderEventExt, PushTransactionEventExt},
            xgateway_common::{
                BtcTrusteeAddrInfo, Chain, TrusteeSessionInfoLenStoreExt,
                TrusteeSessionInfoOfStoreExt,
            },
            xsystem::{NetworkPropsStoreExt, NetworkType as ChainXNetwork},
        },
        primitives::{AccountId, Address, Balance},
        ChainXNodeRuntime, ChainXPairSigner,
    },
};

const TIMEOUT: u64 = 10; // 10 seconds

#[derive(Clone)]
pub struct ChainX {
    pub client: Client<ChainXNodeRuntime>,

    pub chainx_network: ChainXNetwork,
    pub btc_network: BtcNetwork,
    pub btc_min_deposit: u64,
    pub btc_withdrawal_fee: u64,
    // current hot and cold multisig btc address for detecting withdraw, deposit and hot-and-cold
    pub current_trustee_pair: Option<(BtcAddress, BtcAddress)>,
    // previous hot and cold multisig btc address for detecting trustee transition.
    pub previous_trustee_pair: Option<(BtcAddress, BtcAddress)>,
}

impl ChainX {
    pub async fn new<U: Into<String>>(url: U) -> Result<Self, Error> {
        let client = ClientBuilder::new().set_url(url).build().await?;

        let (chainx_network, btc_network, btc_min_deposit, btc_withdrawal_fee) = future::join4(
            client.network_props(None),
            client.network_id(None),
            client.btc_min_deposit(None),
            client.btc_withdrawal_fee(None),
        )
        .await;

        let chainx_network = chainx_network?;
        let btc_network = btc_network?;
        let btc_min_deposit = btc_min_deposit?;
        let btc_withdrawal_fee = btc_withdrawal_fee?;

        info!(
            "[ChainX|new] ChainX Info: [\
                ChainX Network: {:?}, ChainX Genesis: {:?}, \
                Bitcoin Network: {:?}, Bitcoin Min Deposit: {:?}, Bitcoin Withdrawal Fee: {:?}\
            ]",
            chainx_network,
            client.genesis(),
            btc_network,
            btc_min_deposit,
            btc_withdrawal_fee
        );
        assert_eq!(chainx_network, ChainXNetwork::Testnet);
        // assert_eq!(btc_network, BtcNetwork::Mainnet);
        // assert_eq!(btc_min_deposit, 100_000); // 0.001 BTC
        // assert_eq!(btc_withdrawal_fee, 500_000); // 0.005 BTC

        Ok(Self {
            client,
            chainx_network,
            btc_network,
            btc_min_deposit,
            btc_withdrawal_fee,
            current_trustee_pair: None,
            previous_trustee_pair: None,
        })
    }

    pub async fn update_trustee_pair(&mut self) -> Result<(), Error> {
        let trustee_session_info_len: u32 = self
            .client
            .trustee_session_info_len(&Chain::Bitcoin, None)
            .await?;

        let current_trustee_session_number = trustee_session_info_len
            .checked_sub(1)
            .unwrap_or(u32::max_value());
        let current_trustee_pair = self
            .btc_trustee_pair(current_trustee_session_number)
            .await?;

        let previous_trustee_session_number = trustee_session_info_len
            .checked_sub(2)
            .unwrap_or(u32::max_value());
        let previous_trustee_pair = self
            .btc_trustee_pair(previous_trustee_session_number)
            .await?;

        self.current_trustee_pair = current_trustee_pair;
        self.previous_trustee_pair = previous_trustee_pair;
        Ok(())
    }

    pub async fn btc_trustee_pair(
        &self,
        session_number: u32,
    ) -> Result<Option<(BtcAddress, BtcAddress)>, Error> {
        if let Some(trustee_session_info) = self
            .client
            .trustee_session_info_of(&Chain::Bitcoin, &session_number, None)
            .await?
        {
            let hot_addr: Vec<u8> = trustee_session_info.0.hot_address;
            let hot_addr = BtcTrusteeAddrInfo::try_from(hot_addr)?;
            let hot_addr = String::from_utf8(hot_addr.addr)?.parse::<BtcAddress>()?;
            let cold_addr: Vec<u8> = trustee_session_info.0.cold_address;
            let cold_addr = BtcTrusteeAddrInfo::try_from(cold_addr)?;
            let cold_addr = String::from_utf8(cold_addr.addr)?.parse::<BtcAddress>()?;
            info!(
                "[ChainX|btc_trustee_pair] ChainX X-BTC Trustee Session Info (session number = {:?}): \
                [Hot Address: {}, Cold Address: {}, Threshold: {}, Trustee List: {:?}]",
                session_number, hot_addr, cold_addr, trustee_session_info.0.threshold, trustee_session_info.0.trustee_list
            );
            Ok(Some((hot_addr, cold_addr)))
        } else {
            Ok(None)
        }
    }

    pub async fn free_pcx_balance(&self, account_id: &AccountId) -> Result<Balance, Error> {
        let account_info: AccountInfo<ChainXNodeRuntime> =
            self.client.account(&account_id, None).await?;
        let account_data: AccountData<Balance> = account_info.data;
        let free = account_data.free;
        info!(
            "[ChainX|free_pcx_balance] `{:?}` PCX Free = {:?}",
            account_id, free
        );
        // Less than 0.5 PCX in the account
        if free < 50_000_000 {
            warn!("`{:?}` PCX Free < 0.5 PCX", account_id);
            return Err(Error::Other(format!(
                "Free PCX Balance of `{:?}` < 0.5",
                account_id
            )));
        }
        Ok(free)
    }

    pub async fn btc_best_index(&self) -> Result<BtcHeaderIndex, Error> {
        let best_index = self.client.best_index(None).await?;
        info!(
            "[ChainX|btc_best_index] Height #{}, Hash: {:?}",
            best_index.height,
            hash_rev(best_index.hash)
        );
        Ok(best_index)
    }

    pub async fn btc_confirmed_index(&self) -> Result<BtcHeaderIndex, Error> {
        match self.client.confirmed_index(None).await? {
            Some(confirmed_index) => {
                info!(
                    "[ChainX|btc_confirmed_index] Height #{}, Hash: {:?}",
                    confirmed_index.height,
                    hash_rev(confirmed_index.hash)
                );
                Ok(confirmed_index)
            }
            None => {
                // only use for the initialized confirmed index of the ChainX network.
                let genesis: (BtcBlockHeader, u32) = self.client.genesis_info(None).await?;
                let confirmed_index = BtcHeaderIndex {
                    hash: genesis.0.hash(),
                    height: genesis.1,
                };
                info!(
                    "[ChainX|btc_confirmed_index] (From genesis) Height #{}, Hash: {:?}",
                    confirmed_index.height,
                    hash_rev(confirmed_index.hash)
                );
                Ok(confirmed_index)
            }
        }
    }

    pub async fn btc_block_hash_for(&self, height: u32) -> Result<Vec<H256>, Error> {
        let hashes = self.client.block_hash_for(height, None).await?;
        info!(
            "[ChainX|btc_block_hash_for] Height #{}, Hashes: {:?}",
            height,
            hashes
                .iter()
                .map(|hash| hash_rev(*hash))
                .collect::<Vec<_>>()
        );
        Ok(hashes)
    }

    pub async fn btc_block_header(
        &self,
        block_hash: &H256,
    ) -> Result<Option<BtcHeaderInfo>, Error> {
        if let Some(header) = time::timeout(
            Duration::from_secs(TIMEOUT),
            self.client.headers(block_hash, None),
        )
        .await??
        {
            info!(
                "[ChainX|btc_block_header] Height #{}, Header: {:?}",
                header.height, header.header,
            );
            Ok(Some(header))
        } else {
            Ok(None)
        }
    }

    pub async fn btc_best_block_header(&self) -> Result<Option<BtcHeaderInfo>, Error> {
        let best_index = self.btc_best_index().await?;
        self.btc_block_header(&best_index.hash).await
    }

    pub async fn btc_withdrawal_proposal(
        &self,
    ) -> Result<Option<BtcWithdrawalProposal<AccountId>>, Error> {
        let withdrawal_proposal: Option<BtcWithdrawalProposal<AccountId>> =
            self.client.withdrawal_proposal(None).await?;
        if let Some(ref withdrawal_proposal) = withdrawal_proposal {
            info!(
                "[ChainX|btc_withdrawal_proposal] BTC Withdrawal Proposal: {:?}",
                withdrawal_proposal
            );
        }
        Ok(withdrawal_proposal)
    }

    pub async fn btc_pending_deposits<T: AsRef<[u8]>>(
        &self,
        btc_address: T,
    ) -> Result<Vec<BtcDepositCache>, Error> {
        let btc_address = btc_address.as_ref();
        let deposit_cache: Vec<BtcDepositCache> =
            self.client.pending_deposits(btc_address, None).await?;
        info!(
            "[ChainX|btc_pending_deposits] BTC Address `{}` ==> BTC Deposit Cache: {:?}",
            hex::encode(btc_address),
            deposit_cache
        );
        Ok(deposit_cache)
    }

    pub async fn btc_genesis_info(&self) -> Result<(BtcBlockHeader, u32), Error> {
        let genesis: (BtcBlockHeader, u32) = self.client.genesis_info(None).await?;
        info!(
            "[ChainX|btc_genesis_info] BTC Block Height #{} ({:?})",
            genesis.1,
            hash_rev(genesis.0.hash())
        );
        Ok(genesis)
    }

    pub async fn transfer(
        &self,
        signer: &ChainXPairSigner,
        dest: &Address,
        amount: Balance,
    ) -> Result<(), Error> {
        info!(
            "[ChainX|transfer] From: {:?}, To: {:?}, Amount: {}",
            signer.account_id(),
            dest,
            amount,
        );
        let ext: ExtrinsicSuccess<ChainXNodeRuntime> =
            self.client.transfer_and_watch(signer, dest, amount).await?;
        info!(
            "[ChainX|transfer] Extrinsic Block Hash: {:?}, Extrinsic Hash: {:?}",
            ext.block, ext.extrinsic
        );
        if let Some(transfer_event) = ext.transfer()? {
            info!("[ChainX|transfer] Event: {:?}", transfer_event);
            Ok(())
        } else {
            error!("[ChainX|transfer] No Transfer Event");
            Err(Error::Other("Cannot find `Transfer` event".into()))
        }
    }

    pub async fn push_btc_header(
        &self,
        signer: &ChainXPairSigner,
        header: &BtcBlockHeader,
    ) -> Result<(), Error> {
        info!(
            "[ChainX|push_btc_header] Btc Header Hash: {:?}",
            hash_rev(header.hash())
        );

        let header = serialize(header).take();
        let ext: ExtrinsicSuccess<ChainXNodeRuntime> = time::timeout(
            Duration::from_secs(TIMEOUT),
            self.client.push_header_and_watch(signer, &header),
        )
        .await??;
        info!(
            "[ChainX|push_btc_header] Extrinsic Block Hash: {:?}, Extrinsic Hash: {:?}",
            ext.block, ext.extrinsic
        );
        if let Some(push_header_event) = ext.push_header()? {
            info!("[ChainX|push_btc_header] Event: {:?}", push_header_event);
            Ok(())
        } else {
            error!("[ChainX|push_btc_header] No PushHeader Event");
            Err(Error::Other("Cannot find `PushHeader` event".into()))
        }
    }

    pub async fn push_btc_transaction(
        &self,
        signer: &ChainXPairSigner,
        tx: &BtcTransaction,
        relayed_info: &BtcRelayedTxInfo,
        prev_tx: &Option<BtcTransaction>,
    ) -> Result<(), Error> {
        let tx_hash = hash_rev(tx.hash());
        let prev_tx_hash = prev_tx.as_ref().map(|prev_tx| hash_rev(prev_tx.hash()));
        let block_hash = hash_rev(relayed_info.block_hash);
        let merkle_proof = serialize(&relayed_info.merkle_proof);
        info!(
            "[ChainX|push_btc_transaction] Tx: {:?}, Prev Tx: {:?}, Block: {:?}, Merkle Proof: {:?}",
            tx_hash, prev_tx_hash, block_hash, merkle_proof
        );

        let tx = serialize(tx).take();
        let prev_tx = prev_tx.as_ref().map(|prev_tx| serialize(prev_tx).take());
        let ext: ExtrinsicSuccess<ChainXNodeRuntime> = time::timeout(
            Duration::from_secs(TIMEOUT),
            self.client
                .push_transaction_and_watch(signer, &tx, relayed_info, &prev_tx),
        )
        .await??;
        info!(
            "[ChainX|push_btc_transaction] Extrinsic Block Hash: {:?}, Extrinsic Hash: {:?}",
            ext.block, ext.extrinsic,
        );
        if let Some(push_tx_event) = ext.push_transaction()? {
            info!("[ChainX|push_btc_transaction] Event: {:?}", push_tx_event);
            Ok(())
        } else {
            error!("[ChainX|push_btc_transaction] No PushTransaction Event");
            Err(Error::Other("Cannot find `PushTransaction` event".into()))
        }
    }
}

impl ChainX {
    pub fn detect_xbtc_tx(&self, input_addr: Option<BtcAddress>, tx: &BtcTransaction) -> BtcTxType {
        // Detect X-BTC `Withdrawal`/`HotAndCold`/`TrusteeTransition` transaction
        if let Some(input_addr) = input_addr {
            let current_trustee_pair = self.current_trustee_pair.unwrap();

            let all_outputs_is_trustee = tx
                .outputs
                .iter()
                .map(|output| extract_output_addr(output, self.btc_network).unwrap_or_default())
                .all(|addr| is_trustee_addr(addr, current_trustee_pair));

            if is_trustee_addr(input_addr, current_trustee_pair) {
                return if all_outputs_is_trustee {
                    BtcTxType::HotAndCold
                } else {
                    BtcTxType::Withdrawal
                };
            }
            if let Some(previous_trustee_pair) = self.previous_trustee_pair {
                if is_trustee_addr(input_addr, previous_trustee_pair) && all_outputs_is_trustee {
                    return BtcTxType::TrusteeTransition;
                }
            }
        }

        // Detect X-BTC `Deposit` transaction
        self.detect_xbtc_deposit_tx(input_addr, tx)
    }

    // Detect X-BTC `Deposit` transaction
    // The outputs of X-BTC `Deposit` transaction must be in the following format (ignore output order):
    // - 2 outputs (e.g. tx e3639343ca806fe3bf2513971b79130eef88aa05000ce538c6af199dd8ef3ca7):
    //   -- Null data transaction
    //   -- X-BTC hot trustee address (deposit value)
    // - 3 outputs (e.g. tx baeb271dde259a0e488705f732947e4d03dbb680b1f2d3fa0bf624644b3fad50):
    //   -- Null data transaction
    //   -- X-BTC hot trustee address (deposit value)
    //   -- Change address (don't care)
    pub fn detect_xbtc_deposit_tx(
        &self,
        input_addr: Option<BtcAddress>,
        tx: &BtcTransaction,
    ) -> BtcTxType {
        // The numbers of outputs must be 2 or 3.
        if tx.outputs.len() != 2 && tx.outputs.len() != 3 {
            return BtcTxType::Irrelevance;
        }

        // only handle first valid account info opreturn, other opreturn would drop
        let opreturn_script = tx
            .outputs
            .iter()
            .filter_map(|output| {
                let script = BtcScript::new(output.script_pubkey.clone());
                if script.is_null_data_script() {
                    Some(script)
                } else {
                    None
                }
            })
            .take(1)
            .next();

        for output in &tx.outputs {
            // extract destination address from the script.
            if let Some(dest_addr) = extract_output_addr(output, self.btc_network) {
                // check if the script address of the output is the hot trustee address
                // and if deposit value is greater than minimum deposit value.
                let (hot_addr, _) = self.current_trustee_pair.unwrap();
                let deposit_value = output.value;
                if dest_addr.hash == hot_addr.hash && deposit_value >= self.btc_min_deposit {
                    let account_info = opreturn_script
                        .and_then(|script| parse_opreturn(&script))
                        .and_then(|opreturn| handle_opreturn(&opreturn));
                    return BtcTxType::Deposit(DepositInfo {
                        deposit_value,
                        op_return: account_info,
                        input_addr,
                    });
                }
            }
        }
        BtcTxType::Irrelevance
    }
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub enum BtcTxType {
    Withdrawal,
    Deposit(DepositInfo),
    HotAndCold,
    TrusteeTransition,
    Irrelevance,
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct DepositInfo {
    pub deposit_value: u64,
    pub op_return: Option<(AccountId, Option<String>)>,
    pub input_addr: Option<BtcAddress>,
}

fn is_trustee_addr(addr: BtcAddress, trustee_pair: (BtcAddress, BtcAddress)) -> bool {
    let (hot_addr, cold_addr) = trustee_pair;
    addr.hash == hot_addr.hash || addr.hash == cold_addr.hash
}

// only support `p2pk`, `p2pkh` and `p2sh`.
pub fn extract_output_addr(
    output: &BtcTransactionOutput,
    network: BtcNetwork,
) -> Option<BtcAddress> {
    let script = BtcScript::new(output.script_pubkey.clone());
    let script_type = script.script_type();
    match script_type {
        BtcScriptType::PubKey | BtcScriptType::PubKeyHash | BtcScriptType::ScriptHash => {
            let script_addresses = script
                .extract_destinations()
                .map_err(|err| {
                    error!(
                        "[extract_output_addr] cannot extract destinations of btc script, type = {:?}, script: {}, err: {}",
                        script_type, script, err
                    );
                }).unwrap_or_default();

            // find address in this transaction
            if script_addresses.len() == 1 {
                let address = &script_addresses[0];
                Some(BtcAddress {
                    kind: address.kind,
                    network,
                    hash: address.hash,
                })
            } else {
                warn!(
                    "[extract_output_addr] cannot extract address of btc script, type = {:?}, address: {:?}, script: {}",
                    script_type, script_addresses, script
                );
                None
            }
        }
        _ => None,
    }
}

// Parse btc script and return the opreturn data.
// OP_RETURN format:
// - op_return + op_push(<0x4c) + data (op_push == data.len())
// - op_return + op_push(=0x4c) + data.len() + data
fn parse_opreturn(script: &BtcScript) -> Option<Vec<u8>> {
    if !script.is_null_data_script() {
        return None;
    }

    // jump `OP_RETURN`, after checking `is_null_data_script`
    // subscript = `op_push + data` or `op_push + data.len() + data`
    let subscript = script.subscript(1);
    // parse op_push and data.
    let op_push = subscript[0];
    match op_push.cmp(&(BtcScriptOpcode::OP_PUSHDATA1 as u8)) {
        Ordering::Less => {
            if subscript.len() < 2 {
                error!(
                    "[parse_opreturn] nothing after `OP_PUSHDATA1`, invalid opreturn script: {:?}",
                    script
                );
                return None;
            }
            let data = &subscript[1..];
            if op_push as usize == data.len() {
                Some(data.to_vec())
            } else {
                error!(
                    "[parse_opreturn] unexpected opreturn source error, expected data len: {}, actual data: {}",
                    op_push, hex::encode(data)
                );
                None
            }
        }
        Ordering::Equal => {
            // if op_push == `OP_PUSHDATA1`, we must have extra byte for the length of data, or it's an invalid data.
            if subscript.len() < 3 {
                error!(
                    "[parse_opreturn] nothing after `OP_PUSHDATA1`, invalid opreturn script: {:?}",
                    script
                );
                return None;
            }
            let data_len = subscript[1];
            let data = &subscript[2..];
            if data_len as usize == data.len() {
                Some(data.to_vec())
            } else {
                error!(
                    "[parse_opreturn] unexpected opreturn source error, expected data len: {}, actual data: {}",
                    data_len, hex::encode(data)
                );
                None
            }
        }
        Ordering::Greater => {
            error!(
                "[parse_opreturn] unexpected opreturn source error, \
                opreturn format should be `op_return+op_push+data` or `op_return+op_push+data_len+data`, \
                op_push: {:?}", op_push
            );
            None
        }
    }
}

// Handle the opreturn bytes and return the ChainX AccountId and Channel Name.
fn handle_opreturn(opreturn: &[u8]) -> Option<(AccountId, Option<String>)> {
    let v = opreturn
        .split(|x| *x == b'@')
        .map(|d| d.to_vec())
        .collect::<Vec<_>>();

    if v.len() != 1 && v.len() != 2 {
        error!(
            "[parse_opreturn] Cannot parse opreturn, opreturn data should be `account` or `account@channel_name`, but actual opreturn: {}",
            hex::encode(opreturn)
        );
        return None;
    }

    // Check if the account is a ChainX Account
    let account = String::from_utf8_lossy(&v[0]);
    let (account, format) = AccountId::from_ss58check_with_version(account.as_ref()).ok()?;
    if format != Ss58AddressFormat::ChainXAccount {
        error!(
            "[handle_opreturn] Account format ss58 check successfully, but `{:?} (format = {:?})` is not ChainX Account",
            account, format
        );
        return None;
    }

    // channel_name is a validator
    let channel_name = if v.len() == 2 {
        Some(String::from_utf8_lossy(&v[1]).into_owned())
    } else {
        None
    };

    debug!(
        "[handle_opreturn] Parse opreturn success, ChainX Account: {:?}, Channel: {:?}",
        account, channel_name
    );
    Some((account, channel_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{get_account_id_from_seed, get_pair_from_seed};

    // use your own chainx node config.
    const CHAINX_WS_URL: &str = "ws://127.0.0.1:8087";

    #[test]
    fn test_account() {
        let alice = get_account_id_from_seed("Alice");
        println!("Alice = {:?}, {}", alice, alice);
        let bob = get_account_id_from_seed("Bob");
        println!("Bob = {:?}, {}", bob, bob);
        let charlie = get_account_id_from_seed("Charlie");
        println!("Charlie = {:?}, {}", charlie, charlie);

        // xgatewaycommon_bitcoinGenerateTrusteeSessionInfo
        let hot_addr = "3Cg16oUAzxj5EzpaHX6HHJUpJnuctEb9L8"
            .parse::<BtcAddress>()
            .unwrap();
        let cold_addr = "3H7Gu3KsGoa8UbqrY5hfA2S3PVsPwzur3t"
            .parse::<BtcAddress>()
            .unwrap();
        println!("hot: {:?}, cold: {:?}", hot_addr, cold_addr);
    }

    #[test]
    fn test_parse_and_handle_opreturn() {
        // deposit tx: 928bfd32f65317f51c91609f731fc092f4bad88dbe08ab77a8d7b4758e49080c
        // null data script: 6a 30 3552514b6465416535645a64555179414b7841784e64464d746352534b714b4639564e7a6270524b5458536975483557
        let bytes = hex::decode("6a303552514b6465416535645a64555179414b7841784e64464d746352534b714b4639564e7a6270524b5458536975483557").unwrap();
        let script = BtcScript::new(bytes.into());
        let opreturn = parse_opreturn(&script).unwrap();
        assert_eq!(
            opreturn,
            b"5RQKdeAe5dZdUQyAKxAxNdFMtcRSKqKF9VNzbpRKTXSiuH5W".to_vec()
        );
        let (account, channel) = handle_opreturn(&opreturn).unwrap();
        assert_eq!(
            account,
            AccountId::from_ss58check("5RQKdeAe5dZdUQyAKxAxNdFMtcRSKqKF9VNzbpRKTXSiuH5W").unwrap()
        );
        assert_eq!(channel, None);

        // deposit tx: 28cc1f0563713aa4fae7fd279f06c1567cd16255b639f316a0189662f5d117af
        // null data script: 6a 36 355273484763425670656869354d5732346e4c62386170364136336d436a6a724e4a3169384259335273346f63733233404a696e6d61
        let bytes = hex::decode("6a36355273484763425670656869354d5732346e4c62386170364136336d436a6a724e4a3169384259335273346f63733233404a696e6d61").unwrap();
        let script = BtcScript::new(bytes.into());
        let opreturn = parse_opreturn(&script).unwrap();
        assert_eq!(
            opreturn,
            b"5RsHGcBVpehi5MW24nLb8ap6A63mCjjrNJ1i8BY3Rs4ocs23@Jinma".to_vec()
        );
        let (account, channel) = handle_opreturn(&opreturn).unwrap();
        assert_eq!(
            account,
            AccountId::from_ss58check("5RsHGcBVpehi5MW24nLb8ap6A63mCjjrNJ1i8BY3Rs4ocs23").unwrap()
        );
        assert_eq!(channel, Some("Jinma".into()));
    }

    #[ignore]
    #[tokio::test]
    async fn test_chainx() {
        let chainx = ChainX::new(CHAINX_WS_URL).await.unwrap();

        let _btc_withdrawal_proposal = chainx.btc_withdrawal_proposal().await.unwrap();

        let alice = get_account_id_from_seed("Alice");
        let _free_pcx = chainx.free_pcx_balance(&alice).await.unwrap();

        let index = chainx.btc_best_index().await.unwrap();
        println!(
            "Best Index: height {:?}, hash {:?}",
            index.height,
            hash_rev(index.hash)
        );
        // Height #576576, Hash: 0x82185fa131e2e2e1ddf05125a0950271b088eb8df52117000000000000000000
        let index = chainx.btc_confirmed_index().await.unwrap();
        println!(
            "Confirmed Index: height {:?} hash {:?}",
            index.height,
            hash_rev(index.hash)
        );
        let hashes = chainx.btc_block_hash_for(1863320).await.unwrap();
        println!(
            "Block Hash For: {:?}",
            hashes.into_iter().map(hash_rev).collect::<Vec<_>>()
        );
        // Height #576576, Hash: 0x82185fa131e2e2e1ddf05125a0950271b088eb8df52117000000000000000000
        let header = chainx.btc_best_block_header().await.unwrap();
        println!("Best Block Header: {:?}", header);
        // Height #576576, Header: BlockHeader { version: 536870912, previous_header_hash: 0x0000000000000000000a4adf6c5192128535d4dcb56cfb5753755f8d392b26bf, merkle_root_hash: 0x1d21e60acb0b12e5cfd3f775edb647f982a2d666f9886b2f61ea5e72577b0f5e, time: 1558168296, bits: Compact(388627269), nonce: 1439505020 }
        let btc_genesis = chainx.btc_genesis_info().await.unwrap();
        println!("Bitcoin Genesis: {:?}", btc_genesis);
        // Height #576576, Header: BlockHeader { version: 536870912, previous_header_hash: 0x0000000000000000000a4adf6c5192128535d4dcb56cfb5753755f8d392b26bf, merkle_root_hash: 0x1d21e60acb0b12e5cfd3f775edb647f982a2d666f9886b2f61ea5e72577b0f5e, time: 1558168296, bits: Compact(388627269), nonce: 1439505020 }
    }

    #[ignore]
    #[tokio::test]
    async fn test_btc_trustee() {
        let chainx = ChainX::new(CHAINX_WS_URL).await.unwrap();
        let pair = chainx.btc_trustee_pair(0).await.unwrap();
        println!("Bitcoin Trustee Pair: {:?}", pair);
    }

    #[ignore]
    #[tokio::test]
    async fn test_transfer() {
        let chainx = ChainX::new(CHAINX_WS_URL).await.unwrap();

        let alice = get_account_id_from_seed("Alice");
        let bob = get_account_id_from_seed("Bob");

        let alice_before = chainx.free_pcx_balance(&alice).await.unwrap();
        let bob_before = chainx.free_pcx_balance(&bob).await.unwrap();
        println!("Alice = {}, Bob = {}", alice_before, bob_before);

        // transfer (Alice ==> Bob)
        let pair = get_pair_from_seed("Alice");
        let signer = ChainXPairSigner::new(pair);
        let amount = 10_000;
        let dest = get_account_id_from_seed("Bob").into();
        chainx.transfer(&signer, &dest, amount).await.unwrap();

        let alice_after = chainx.free_pcx_balance(&alice).await.unwrap();
        let bob_after = chainx.free_pcx_balance(&bob).await.unwrap();
        println!("Alice = {}, Bob = {}", alice_after, bob_after);

        assert!(alice_before - amount >= alice_after);
        assert_eq!(bob_before + amount, bob_after);
    }
}
