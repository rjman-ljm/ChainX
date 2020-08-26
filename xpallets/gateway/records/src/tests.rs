// Copyright 2018-2019 Chainpool.

use super::mock::*;
use super::*;

use frame_support::{assert_noop, assert_ok};

#[test]
fn test_normal() {
    ExtBuilder::default().build_and_execute(|| {
        // deposit
        assert_ok!(XRecords::deposit(&ALICE, &X_BTC, 100));
        assert_eq!(XAssets::usable_balance(&ALICE, &X_BTC), 100 + 100);

        // withdraw
        assert_ok!(XRecords::withdrawal(
            &ALICE,
            &X_BTC,
            50,
            b"addr".to_vec(),
            b"ext".to_vec().into()
        ));

        let numbers = XRecords::withdrawals_list_by_chain(Chain::Bitcoin)
            .into_iter()
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        assert_eq!(numbers.len(), 1);

        assert_ok!(XRecords::process_withdrawals(Chain::Bitcoin, &numbers));
        for i in numbers {
            assert_ok!(XRecords::finish_withdrawal(None, i));
        }
        assert_eq!(XAssets::usable_balance(&ALICE, &X_BTC), 50 + 100);
    })
}

#[test]
fn test_normal2() {
    ExtBuilder::default().build_and_execute(|| {
        // deposit
        assert_ok!(XRecords::deposit(&ALICE, &X_BTC, 100));
        assert_eq!(XAssets::usable_balance(&ALICE, &X_BTC), 100 + 100);
        assert_ok!(XRecords::deposit(&ALICE, &X_ETH, 500));
        assert_eq!(XAssets::usable_balance(&ALICE, &X_ETH), 500 + 100);

        // withdraw
        assert_ok!(XRecords::withdrawal(
            &ALICE,
            &X_BTC,
            50,
            b"addr".to_vec(),
            b"ext".to_vec().into()
        ));
        // withdrawal twice at once
        assert_ok!(XRecords::withdrawal(
            &ALICE,
            &X_ETH,
            100,
            b"addr".to_vec(),
            b"ext".to_vec().into()
        ));
        assert_ok!(XRecords::withdrawal(
            &ALICE,
            &X_ETH,
            50,
            b"addr".to_vec(),
            b"ext".to_vec().into()
        ));

        let numbers1 = XRecords::withdrawals_list_by_chain(Chain::Bitcoin)
            .into_iter()
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        assert_eq!(numbers1.len(), 1);

        let numbers2 = XRecords::withdrawals_list_by_chain(Chain::Ethereum)
            .into_iter()
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        assert_eq!(numbers2.len(), 2);

        let mut wrong_numbers = numbers1.clone();
        wrong_numbers.extend_from_slice(&numbers2);

        assert_noop!(
            XRecords::process_withdrawals(Chain::Bitcoin, &wrong_numbers),
            XRecordsErr::UnexpectedChain
        );
        assert_ok!(XRecords::process_withdrawals(Chain::Bitcoin, &numbers1));
        assert_ok!(XRecords::process_withdrawals(Chain::Ethereum, &numbers2));

        assert_ok!(XRecords::finish_withdrawals(
            Some(Chain::Bitcoin),
            &numbers1
        ));
        assert_ok!(XRecords::finish_withdrawals(
            Some(Chain::Ethereum),
            &numbers2
        ));

        assert_eq!(XAssets::usable_balance(&ALICE, &X_BTC), 50 + 100);
        assert_eq!(
            XAssets::usable_balance(&ALICE, &X_ETH),
            500 + 100 - 50 - 100
        );
    })
}

#[test]
fn test_withdrawal_more_then_usable() {
    ExtBuilder::default().build_and_execute(|| {
        assert_ok!(XRecords::deposit(&ALICE, &X_BTC, 10));

        assert_noop!(
            XRecords::withdrawal(
                &ALICE,
                &X_BTC,
                100 + 50,
                b"addr".to_vec(),
                b"ext".to_vec().into()
            ),
            xpallet_assets::Error::<Test>::InsufficientBalance
        );
    })
}

#[test]
fn test_withdrawal_chainx() {
    ExtBuilder::default().build_and_execute(|| {
        assert_noop!(
            XRecords::deposit(&ALICE, &ChainXAssetId::get(), 10),
            xpallet_assets::Error::<Test>::DenyNativeAsset
        );

        assert_noop!(
            XRecords::withdrawal(
                &ALICE,
                &ChainXAssetId::get(),
                50,
                b"addr".to_vec(),
                b"ext".to_vec().into()
            ),
            xpallet_assets::Error::<Test>::DenyNativeAsset
        );
    })
}