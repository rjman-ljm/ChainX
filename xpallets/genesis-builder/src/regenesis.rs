use crate::{GenesisConfig, Trait};

mod balances {
    use frame_support::{traits::StoredMap, StorageValue};
    use pallet_balances::AccountData;
    use xp_genesis_builder::FreeBalanceInfo;

    use crate::Trait;

    // Set PCX free balance.
    pub fn initialize<T: Trait>(free_balances: &[FreeBalanceInfo<T::AccountId, T::Balance>]) {
        let set_free_balance = |who: &T::AccountId, free: &T::Balance| {
            T::AccountStore::insert(
                who,
                AccountData {
                    free: *free,
                    ..Default::default()
                },
            )
        };

        let mut total_issuance = T::Balance::default();

        for FreeBalanceInfo { who, free } in free_balances {
            set_free_balance(who, free);
            total_issuance += *free;
        }

        pallet_balances::TotalIssuance::<T>::mutate(|v| *v = total_issuance);
    }
}

mod xassets {
    // Set XBTC free balance.
    use xp_genesis_builder::FreeBalanceInfo;
    use xp_protocol::X_BTC;

    use super::*;
    use crate::AssetBalanceOf;

    pub fn initialize<T: Trait>(xbtc_assets: &[FreeBalanceInfo<T::AccountId, AssetBalanceOf<T>>]) {
        for FreeBalanceInfo { who, free } in xbtc_assets {
            xpallet_assets::Module::<T>::force_set_free_balance(&X_BTC, who, *free);
        }
    }
}

mod xstaking {
    use xp_genesis_builder::{Nomination, NominatorInfo, XStakingParams};

    use super::*;
    use crate::StakingBalanceOf;

    // Simulate the bond operation.
    pub fn initialize<T: Trait>(
        params: &XStakingParams<T::AccountId, StakingBalanceOf<T>>,
        initial_authorities: &[Vec<u8>],
    ) {
        let XStakingParams {
            validators,
            nominators,
        } = params;

        // Firstly register the genesis validators.
        xpallet_mining_staking::Module::<T>::initialize_validators(validators, initial_authorities)
            .expect("Failed to initialize genesis staking validators");

        // Then mock the validator bond themselves and set the vote weights.
        for NominatorInfo {
            nominator,
            nominations,
        } in nominators
        {
            for Nomination {
                nominee,
                nomination,
            } in nominations
            {
                xpallet_mining_staking::Module::<T>::force_set_nominator_vote_weight(
                    nominator,
                    nominee,
                    Default::default(),
                );
                xpallet_mining_staking::Module::<T>::force_bond(nominator, nominee, *nomination)
                    .expect("force validator self-bond can not fail; qed");
            }
        }
    }
}

mod xmining_asset {
    use xp_genesis_builder::FreeBalanceInfo;
    use xp_protocol::X_BTC;

    use super::*;
    use crate::AssetBalanceOf;

    // Set the weight related to zero.
    pub fn initialize<T: Trait>(xbtc_assets: &[FreeBalanceInfo<T::AccountId, AssetBalanceOf<T>>]) {
        let current_block = frame_system::Module::<T>::block_number();

        for FreeBalanceInfo { who, .. } in xbtc_assets {
            xpallet_mining_asset::Module::<T>::force_set_miner_mining_weight(
                who,
                &X_BTC,
                Default::default(),
                current_block,
            );
        }

        xpallet_mining_asset::Module::<T>::force_set_asset_mining_weight(
            &X_BTC,
            Default::default(),
            current_block,
        );
    }
}

pub(crate) fn initialize<T: Trait>(config: &GenesisConfig<T>) {
    let now = std::time::Instant::now();

    balances::initialize::<T>(&config.params.balances);
    xassets::initialize::<T>(&config.params.xassets);
    xstaking::initialize::<T>(&config.params.xstaking, &config.initial_authorities);
    xmining_asset::initialize::<T>(&config.params.xassets);

    xp_logging::info!(
        "Took {:?}ms to orchestrate the regenesis state",
        now.elapsed().as_millis()
    );
}
