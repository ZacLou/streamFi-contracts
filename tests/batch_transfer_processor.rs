//! Integration tests for
//! `drip_batch_processor::BatchTransferProcessor::process_batch`.
//!
//! The processor pulls `sum(amounts)` from a single `funder` (one inbound
//! transfer, one auth) and fans the funds out to `recipients` in order. All
//! validation runs before any token movement.

use drip_batch_processor::{BatchTransferProcessor, BatchTransferProcessorClient, Error};
use soroban_sdk::{testutils::Address as _, token, Address, Env, Vec};

struct Fixture<'a> {
    env: Env,
    client: BatchTransferProcessorClient<'a>,
    token: token::Client<'a>,
    token_admin_client: token::StellarAssetClient<'a>,
    funder: Address,
}

fn setup<'a>() -> Fixture<'a> {
    let env = Env::default();
    env.mock_all_auths();

    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let id = env.register_contract(None, BatchTransferProcessor);
    let client = BatchTransferProcessorClient::new(&env, &id);

    let funder = Address::generate(&env);

    Fixture {
        token: token::Client::new(&env, &token_addr),
        token_admin_client: token::StellarAssetClient::new(&env, &token_addr),
        client,
        funder,
        env,
    }
}

fn addrs(env: &Env, n: u32) -> Vec<Address> {
    let mut v = Vec::new(env);
    for _ in 0..n {
        v.push_back(Address::generate(env));
    }
    v
}

#[test]
fn process_batch_fans_out_to_every_recipient() {
    let f = setup();
    f.token_admin_client.mint(&f.funder, &100);

    let recipients = addrs(&f.env, 3);
    let amounts = Vec::from_array(&f.env, [10i128, 20, 30]);

    let total = f
        .client
        .process_batch(&f.funder, &f.token.address, &recipients, &amounts);

    assert_eq!(total, 60);
    assert_eq!(f.token.balance(&recipients.get(0).unwrap()), 10);
    assert_eq!(f.token.balance(&recipients.get(1).unwrap()), 20);
    assert_eq!(f.token.balance(&recipients.get(2).unwrap()), 30);
    assert_eq!(f.token.balance(&f.funder), 40);
}

#[test]
fn process_batch_empty_input_is_a_noop() {
    let f = setup();
    let recipients: Vec<Address> = Vec::new(&f.env);
    let amounts: Vec<i128> = Vec::new(&f.env);

    let total = f
        .client
        .process_batch(&f.funder, &f.token.address, &recipients, &amounts);

    assert_eq!(total, 0);
    assert_eq!(f.token.balance(&f.funder), 0);
}

#[test]
fn process_batch_accepts_exactly_100_entries() {
    let f = setup();
    f.token_admin_client.mint(&f.funder, &100);

    let recipients = addrs(&f.env, 100);
    let mut amounts = Vec::new(&f.env);
    for _ in 0..100 {
        amounts.push_back(1i128);
    }

    let total = f
        .client
        .process_batch(&f.funder, &f.token.address, &recipients, &amounts);

    assert_eq!(total, 100);
    assert_eq!(f.token.balance(&f.funder), 0);
}

#[test]
fn process_batch_rejects_101_entries() {
    let f = setup();
    let recipients = addrs(&f.env, 101);
    let mut amounts = Vec::new(&f.env);
    for _ in 0..101 {
        amounts.push_back(1i128);
    }

    assert_eq!(
        f.client
            .try_process_batch(&f.funder, &f.token.address, &recipients, &amounts),
        Err(Ok(Error::BatchTooLarge)),
    );
}

#[test]
fn process_batch_rejects_length_mismatch() {
    let f = setup();
    let recipients = addrs(&f.env, 2);
    let amounts = Vec::from_array(&f.env, [1i128]);

    assert_eq!(
        f.client
            .try_process_batch(&f.funder, &f.token.address, &recipients, &amounts),
        Err(Ok(Error::LengthMismatch)),
    );
}

#[test]
fn process_batch_rejects_zero_amount() {
    let f = setup();
    let recipients = addrs(&f.env, 1);
    let amounts = Vec::from_array(&f.env, [0i128]);

    assert_eq!(
        f.client
            .try_process_batch(&f.funder, &f.token.address, &recipients, &amounts),
        Err(Ok(Error::InvalidAmount)),
    );
}

#[test]
fn process_batch_rejects_zero_amount_in_mixed_batch() {
    let f = setup();
    let recipients = addrs(&f.env, 3);
    let amounts = Vec::from_array(&f.env, [10i128, 0, 30]);

    assert_eq!(
        f.client
            .try_process_batch(&f.funder, &f.token.address, &recipients, &amounts),
        Err(Ok(Error::InvalidAmount)),
    );
}

#[test]
fn process_batch_rejects_negative_amount() {
    let f = setup();
    let recipients = addrs(&f.env, 1);
    let amounts = Vec::from_array(&f.env, [-5i128]);

    assert_eq!(
        f.client
            .try_process_batch(&f.funder, &f.token.address, &recipients, &amounts),
        Err(Ok(Error::InvalidAmount)),
    );
}

#[test]
fn process_batch_detects_total_overflow() {
    let f = setup();
    let recipients = addrs(&f.env, 2);
    let amounts = Vec::from_array(&f.env, [i128::MAX, 1]);

    assert_eq!(
        f.client
            .try_process_batch(&f.funder, &f.token.address, &recipients, &amounts),
        Err(Ok(Error::ArithmeticOverflow)),
    );
}

#[test]
fn process_batch_does_not_move_funds_when_validation_fails() {
    let f = setup();
    f.token_admin_client.mint(&f.funder, &1_000);

    // Length mismatch — must bail before any transfer.
    let recipients = addrs(&f.env, 2);
    let amounts = Vec::from_array(&f.env, [10i128]);
    let _ = f
        .client
        .try_process_batch(&f.funder, &f.token.address, &recipients, &amounts);

    assert_eq!(f.token.balance(&f.funder), 1_000);
}

#[test]
fn error_type_carries_required_traits() {
    fn assert_traits<T: Copy + Clone + core::fmt::Debug + Eq + PartialEq + PartialOrd + Ord>() {}
    assert_traits::<Error>();
    assert_eq!(Error::LengthMismatch as u32, 1);
    assert_eq!(Error::BatchTooLarge as u32, 2);
    assert_eq!(Error::InvalidAmount as u32, 3);
    assert_eq!(Error::ArithmeticOverflow as u32, 4);
}
