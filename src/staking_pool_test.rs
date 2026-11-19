//! Tests for the staking pool yield and unstake accounting (issues #2056-#2058).

#![cfg(test)]

use crate::types::{DataKey, StakingPool};
use crate::{ContractError, QuorumCreditContract, QuorumCreditContractClient};
use soroban_sdk::{
    testutils::Address as _, token::StellarAssetClient, Address, Env, Vec,
};

const DAY: u64 = 24 * 60 * 60;

/// Deploys and initializes the contract with one admin (threshold 1) and a
/// stellar asset token. Returns `(env, contract_id, admin, token)`.
fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let deployer = Address::generate(&env);
    let admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract_id = env.register_contract(None, QuorumCreditContract);

    let client = QuorumCreditContractClient::new(&env, &contract_id);
    client.initialize(
        &deployer,
        &Vec::from_array(&env, [admin.clone()]),
        &1u32,
        &token,
    );

    (env, contract_id, admin, token)
}

fn set_time(env: &Env, timestamp: u64) {
    env.ledger().with_mut(|li| li.timestamp = timestamp);
}

fn set_yield_reserve(env: &Env, contract_id: &Address, amount: i128) {
    env.as_contract(contract_id, || {
        env.storage()
            .persistent()
            .set(&DataKey::YieldReserve, &amount);
    });
}

fn yield_reserve(env: &Env, contract_id: &Address) -> i128 {
    env.as_contract(contract_id, || {
        env.storage()
            .persistent()
            .get(&DataKey::YieldReserve)
            .unwrap_or(0)
    })
}

// ── #2056: claim_yield must not reset the pending-unstake timer ──────────────

#[test]
fn test_claim_yield_does_not_delay_queued_unstake() {
    let (env, contract_id, admin, token) = setup();
    let client = QuorumCreditContractClient::new(&env, &contract_id);
    let signers = Vec::from_array(&env, [admin.clone()]);

    let pool_id = client.create_staking_pool(&signers, &token);

    let staker = Address::generate(&env);
    let stake: i128 = 1_000;
    let asset = StellarAssetClient::new(&env, &token);
    asset.mint(&staker, &stake);
    // The contract must hold enough tokens to pay out the yield below.
    asset.mint(&contract_id, &100);

    let t0: u64 = 1_000_000;
    set_time(&env, t0);
    client.stake_capital(&pool_id, &staker, &stake);

    set_yield_reserve(&env, &contract_id, 10_000);
    client.distribute_yield(&signers, &pool_id, &100);

    // Queue the full exit at t0.
    client.queue_unstake(&pool_id, &staker, &stake);

    // Halfway through the delay the staker claims yield.
    set_time(&env, t0 + DAY / 2);
    let claimed = client.claim_yield(&pool_id, &staker);
    assert_eq!(claimed, 100);

    // 24h after the queue time the withdrawal must still be processable.
    set_time(&env, t0 + DAY);
    let returned = client.process_unstake(&pool_id, &staker);
    assert_eq!(returned, stake);
}
