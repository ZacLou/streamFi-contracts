#![cfg(test)]

extern crate std;

use soroban_sdk::{testutils::Address as _, token, Address, Env, Vec};

use crate::{BatchTransferProcessor, BatchTransferProcessorClient, Error};

fn setup() -> (Env, BatchTransferProcessorClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, BatchTransferProcessor);
    let client = BatchTransferProcessorClient::new(&env, &contract_id);

    let funder = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let tok_admin = token::StellarAssetClient::new(&env, &token);
    tok_admin.mint(&funder, &1_000_000_000i128);

    (env, client, funder, token)
}

#[test]
fn process_batch_success() {
    let (env, client, funder, token) = setup();

    let r1 = Address::generate(&env);
    let r2 = Address::generate(&env);
    let recipients = Vec::from_array(&env, [r1.clone(), r2.clone()]);
    let amounts = Vec::from_array(&env, [100i128, 200i128]);

    let total = client.process_batch(&funder, &token, &recipients, &amounts);
    assert_eq!(total, 300);

    let tk = token::Client::new(&env, &token);
    assert_eq!(tk.balance(&r1), 100);
    assert_eq!(tk.balance(&r2), 200);
}

#[test]
fn process_batch_empty_ok() {
    let (env, client, funder, token) = setup();

    let recipients: Vec<Address> = Vec::new(&env);
    let amounts: Vec<i128> = Vec::new(&env);

    let total = client.process_batch(&funder, &token, &recipients, &amounts);
    assert_eq!(total, 0);
}

#[test]
fn process_batch_rejects_length_mismatch() {
    let (env, client, funder, token) = setup();

    let r1 = Address::generate(&env);
    let recipients = Vec::from_array(&env, [r1.clone()]);
    let amounts = Vec::from_array(&env, [100i128, 200i128]);

    let result = client.try_process_batch(&funder, &token, &recipients, &amounts);
    assert_eq!(result, Err(Ok(Error::LengthMismatch)));
}

#[test]
fn process_batch_rejects_batch_too_large() {
    let (env, client, funder, token) = setup();

    let mut recipients: Vec<Address> = Vec::new(&env);
    let mut amounts: Vec<i128> = Vec::new(&env);
    for _ in 0..101 {
        recipients.push_back(Address::generate(&env));
        amounts.push_back(1i128);
    }

    let result = client.try_process_batch(&funder, &token, &recipients, &amounts);
    assert_eq!(result, Err(Ok(Error::BatchTooLarge)));
}

#[test]
fn process_batch_rejects_invalid_amount() {
    let (env, client, funder, token) = setup();

    let r1 = Address::generate(&env);
    let r2 = Address::generate(&env);
    let recipients = Vec::from_array(&env, [r1.clone(), r2.clone()]);
    let amounts = Vec::from_array(&env, [100i128, 0i128]);

    let result = client.try_process_batch(&funder, &token, &recipients, &amounts);
    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
}

#[test]
fn process_batch_rejects_negative_amount() {
    let (env, client, funder, token) = setup();

    let r1 = Address::generate(&env);
    let recipients = Vec::from_array(&env, [r1.clone()]);
    let amounts = Vec::from_array(&env, [-1i128]);

    let result = client.try_process_batch(&funder, &token, &recipients, &amounts);
    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
}

#[test]
fn process_batch_exactly_max_batch_size_ok() {
    let (env, client, funder, token) = setup();

    let mut recipients: Vec<Address> = Vec::new(&env);
    let mut amounts: Vec<i128> = Vec::new(&env);
    for _ in 0..100 {
        recipients.push_back(Address::generate(&env));
        amounts.push_back(1i128);
    }

    let total = client.process_batch(&funder, &token, &recipients, &amounts);
    assert_eq!(total, 100);
}
