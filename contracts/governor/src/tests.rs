#![cfg(test)]

use super::*;
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events as _},
    Address, Env, IntoVal,
};

fn create_test_env() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let authority = Address::generate(&env);
    let fee_recipient = Address::generate(&env);
    let factory_address = Address::generate(&env);

    (env, authority, fee_recipient, factory_address)
}

#[test]
fn test_set_fee_bps_accepts_valid_values() {
    let (env, authority, fee_recipient, factory_address) = create_test_env();
    let contract_id = env.register_contract(None, DripGovernor);
    let client = DripGovernorClient::new(&env, &contract_id);

    client.initialize(&authority, &fee_recipient, &factory_address);

    // Test valid fee values
    assert!(client.try_set_fee_bps(&authority, &0).is_ok());
    assert!(client.try_set_fee_bps(&authority, &100).is_ok());
    assert!(client.try_set_fee_bps(&authority, &5000).is_ok());
    assert!(client.try_set_fee_bps(&authority, &10_000).is_ok()); // Exactly 100%
}

#[test]
fn test_set_fee_bps_rejects_values_over_10000() {
    let (env, authority, fee_recipient, factory_address) = create_test_env();
    let contract_id = env.register_contract(None, DripGovernor);
    let client = DripGovernorClient::new(&env, &contract_id);

    client.initialize(&authority, &fee_recipient, &factory_address);

    // Test that values over 10,000 (100%) are rejected
    let result = client.try_set_fee_bps(&authority, &10_001);
    assert_eq!(result, Err(Ok(Error::InvalidParam)));

    let result = client.try_set_fee_bps(&authority, &15_000);
    assert_eq!(result, Err(Ok(Error::InvalidParam)));

    let result = client.try_set_fee_bps(&authority, &100_000);
    assert_eq!(result, Err(Ok(Error::InvalidParam)));

    let result = client.try_set_fee_bps(&authority, &u32::MAX);
    assert_eq!(result, Err(Ok(Error::InvalidParam)));
}

#[test]
fn test_set_fee_bps_boundary_values() {
    let (env, authority, fee_recipient, factory_address) = create_test_env();
    let contract_id = env.register_contract(None, DripGovernor);
    let client = DripGovernorClient::new(&env, &contract_id);

    client.initialize(&authority, &fee_recipient, &factory_address);

    // Boundary test: 10,000 should succeed (exactly 100%)
    assert!(client.try_set_fee_bps(&authority, &10_000).is_ok());

    // Boundary test: 10,001 should fail (over 100%)
    let result = client.try_set_fee_bps(&authority, &10_001);
    assert_eq!(result, Err(Ok(Error::InvalidParam)));
}

#[test]
fn test_set_fee_bps_emits_event() {
    let (env, authority, fee_recipient, factory_address) = create_test_env();
    let contract_id = env.register_contract(None, DripGovernor);
    let client = DripGovernorClient::new(&env, &contract_id);

    client.initialize(&authority, &fee_recipient, &factory_address);

    // Set fee and verify event is emitted
    client.set_fee_bps(&authority, &500);

    let events = env.events().all();
    let (_, topics, _data) = events.last().unwrap();

    assert_eq!(
        topics,
        (symbol_short!("fee_bps"), authority.clone()).into_val(&env)
    );
}

#[test]
fn test_set_fee_bps_requires_fee_manager_role() {
    let (env, authority, fee_recipient, factory_address) = create_test_env();
    let contract_id = env.register_contract(None, DripGovernor);
    let client = DripGovernorClient::new(&env, &contract_id);

    client.initialize(&authority, &fee_recipient, &factory_address);

    // Create unauthorized user
    let unauthorized = Address::generate(&env);

    // Should fail without FeeManager role
    let result = client.try_set_fee_bps(&unauthorized, &100);
    assert!(result.is_err());
}

#[test]
fn test_set_fee_bps_blocked_when_paused() {
    let (env, authority, fee_recipient, factory_address) = create_test_env();
    let contract_id = env.register_contract(None, DripGovernor);
    let client = DripGovernorClient::new(&env, &contract_id);

    client.initialize(&authority, &fee_recipient, &factory_address);

    // Pause the governor
    client.governor_pause(&authority);

    // Should fail when paused
    let result = client.try_set_fee_bps(&authority, &100);
    assert_eq!(result, Err(Ok(Error::ContractPaused)));
}
