#![cfg(test)]

use drip_stream::{DripStream, DripStreamClient, Error};
use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    token, Address, Env,
};

// ── Helpers ────────────────────────────────────────────────────────────────

fn base_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set(LedgerInfo {
        timestamp: 1_000_000,
        protocol_version: 21,
        sequence_number: 1,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 16,
        min_persistent_entry_ttl: 4096,
        max_entry_ttl: 6_312_000,
    });
    env
}

fn deploy_funded_stream<'a>(
    env: &'a Env,
    sender: &Address,
    recipient: &Address,
    rate: i128,
    duration: u64,
    clawback: bool,
) -> (DripStreamClient<'a>, Address) {
    let token_admin = Address::generate(env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let tok = token::StellarAssetClient::new(env, &token_addr);
    let deposit = rate * duration as i128;
    tok.mint(sender, &deposit);

    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(env, &stream_id);

    token::Client::new(env, &token_addr).transfer(sender, &stream_id, &deposit);

    let now = env.ledger().timestamp();
    client.initialize(
        sender,
        recipient,
        &token_addr,
        &rate,
        &now,
        &(now + duration),
        &clawback,
        &2_592_000_u64,
    );

    (client, token_addr)
}

/// Read the guard depth counter from instance storage.
fn guard_depth(env: &Env, stream_id: &Address) -> u32 {
    env.as_contract(stream_id, || {
        env.storage()
            .instance()
            .get::<_, u32>(&drip_stream::storage::DataKey::Guard)
            .unwrap_or(0)
    })
}

// ── Lock / unlock lifecycle ─────────────────────────────────────────────────

#[test]
fn test_reentrancy_guard_released_after_successful_withdraw() {
    let env = base_env();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (client, _token_addr) = deploy_funded_stream(&env, &sender, &recipient, 100, 3600, false);

    // Advance time to have some withdrawable balance
    env.ledger().set(LedgerInfo {
        timestamp: env.ledger().timestamp() + 100,
        ..env.ledger().get()
    });

    assert_eq!(guard_depth(&env, &client.address), 0);
    let withdrawn = client.withdraw(&50);
    assert_eq!(withdrawn, 50);
    assert_eq!(guard_depth(&env, &client.address), 0);
}

#[test]
fn test_reentrancy_guard_released_after_withdraw_error() {
    let env = base_env();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (client, _token_addr) = deploy_funded_stream(&env, &sender, &recipient, 100, 3600, false);

    // Withdraw 0 — should fail with InvalidAmount but release the lock
    assert_eq!(guard_depth(&env, &client.address), 0);
    let result = client.try_withdraw(&0);
    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
    assert_eq!(guard_depth(&env, &client.address), 0);
}

#[test]
fn test_reentrancy_guard_released_after_cancel() {
    let env = base_env();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (client, _token_addr) = deploy_funded_stream(&env, &sender, &recipient, 100, 3600, false);

    assert_eq!(guard_depth(&env, &client.address), 0);
    client.cancel(&sender);
    assert_eq!(guard_depth(&env, &client.address), 0);
}

#[test]
fn test_reentrancy_guard_released_after_pause_resume() {
    let env = base_env();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (client, _token_addr) = deploy_funded_stream(&env, &sender, &recipient, 100, 3600, false);

    assert_eq!(guard_depth(&env, &client.address), 0);
    client.pause(&sender);
    assert_eq!(guard_depth(&env, &client.address), 0);
    client.resume(&sender);
    assert_eq!(guard_depth(&env, &client.address), 0);
}

#[test]
fn test_reentrancy_guard_released_after_top_up() {
    let env = base_env();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (client, token_addr) = deploy_funded_stream(&env, &sender, &recipient, 100, 3600, false);

    let tok_admin = token::StellarAssetClient::new(&env, &token_addr);
    tok_admin.mint(&sender, &50_000);

    assert_eq!(guard_depth(&env, &client.address), 0);
    client.top_up(&sender, &50_000);
    assert_eq!(guard_depth(&env, &client.address), 0);
}

#[test]
fn test_reentrancy_guard_released_after_clawback() {
    let env = base_env();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (client, _token_addr) = deploy_funded_stream(&env, &sender, &recipient, 100, 3600, true);

    assert_eq!(guard_depth(&env, &client.address), 0);
    let amount = client.clawback(&sender);
    assert!(amount > 0);
    assert_eq!(guard_depth(&env, &client.address), 0);
}

#[test]
fn test_reentrancy_guard_released_after_force_cancel() {
    let env = base_env();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (client, _token_addr) = deploy_funded_stream(&env, &sender, &recipient, 100, 3600, false);

    client.pause(&sender);

    // Fast-forward 30 days to meet the force-cancel threshold
    env.ledger().set(LedgerInfo {
        timestamp: env.ledger().timestamp() + 2_592_001,
        ..env.ledger().get()
    });

    assert_eq!(guard_depth(&env, &client.address), 0);
    client.force_cancel();
    assert_eq!(guard_depth(&env, &client.address), 0);
}

#[test]
fn test_reentrancy_guard_released_after_transfer_recipient() {
    let env = base_env();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let new_recipient = Address::generate(&env);
    let (client, _token_addr) = deploy_funded_stream(&env, &sender, &recipient, 100, 3600, false);

    assert_eq!(guard_depth(&env, &client.address), 0);
    client.transfer_recipient(&new_recipient);
    assert_eq!(guard_depth(&env, &client.address), 0);
}

// ── Guard blocks when lock is held ──────────────────────────────────────────

#[test]
fn test_reentrancy_guard_blocks_on_manually_held_lock() {
    let env = base_env();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (client, _token_addr) = deploy_funded_stream(&env, &sender, &recipient, 100, 3600, false);

    // Advance time
    env.ledger().set(LedgerInfo {
        timestamp: env.ledger().timestamp() + 100,
        ..env.ledger().get()
    });

    // Manually set the guard depth to the max (simulating a held lock)
    env.as_contract(&client.address, || {
        env.storage()
            .instance()
            .set(&drip_stream::storage::DataKey::Guard, &1_u32);
    });
    assert_eq!(guard_depth(&env, &client.address), 1);

    // Any state-mutating call should fail with ReentrancyForbidden
    let result = client.try_withdraw(&50);
    assert_eq!(result, Err(Ok(Error::ReentrancyForbidden)));

    // The lock should NOT be cleared by a failed attempt
    assert_eq!(guard_depth(&env, &client.address), 1);

    // Manually clear the lock, then operations work again
    env.as_contract(&client.address, || {
        env.storage()
            .instance()
            .set(&drip_stream::storage::DataKey::Guard, &0_u32);
    });
    assert_eq!(guard_depth(&env, &client.address), 0);
    let withdrawn = client.withdraw(&50);
    assert_eq!(withdrawn, 50);
    assert_eq!(guard_depth(&env, &client.address), 0);
}

#[test]
fn test_reentrancy_guard_blocks_all_mutating_operations_when_locked() {
    let env = base_env();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let new_recipient = Address::generate(&env);
    let (client, token_addr) = deploy_funded_stream(&env, &sender, &recipient, 100, 3600, true);

    // Fund a top-up
    let tok_admin = token::StellarAssetClient::new(&env, &token_addr);
    tok_admin.mint(&sender, &50_000);

    env.ledger().set(LedgerInfo {
        timestamp: env.ledger().timestamp() + 100,
        ..env.ledger().get()
    });

    // Manually set guard depth to 1
    env.as_contract(&client.address, || {
        env.storage()
            .instance()
            .set(&drip_stream::storage::DataKey::Guard, &1_u32);
    });

    // All mutating operations must be blocked
    assert_eq!(
        client.try_withdraw(&50),
        Err(Ok(Error::ReentrancyForbidden))
    );
    assert_eq!(
        client.try_cancel(&sender),
        Err(Ok(Error::ReentrancyForbidden))
    );
    assert_eq!(
        client.try_pause(&sender),
        Err(Ok(Error::ReentrancyForbidden))
    );
    assert_eq!(
        client.try_top_up(&sender, &50_000),
        Err(Ok(Error::ReentrancyForbidden))
    );
    assert_eq!(
        client.try_clawback(&sender),
        Err(Ok(Error::ReentrancyForbidden))
    );
    assert_eq!(
        client.try_transfer_recipient(&new_recipient),
        Err(Ok(Error::ReentrancyForbidden))
    );
    assert_eq!(
        client.try_extend_duration(&sender, &3600),
        Err(Ok(Error::ReentrancyForbidden))
    );

    // Read-only operations should still work (they don't use the guard)
    let _ = client.withdrawable();
    let _ = client.streamed_total();
    let _ = client.info();
    let _ = client.event_sequence();
}

// ── Depth counter behavior ──────────────────────────────────────────────────

#[test]
fn test_guard_depth_counter_increments_and_decrements() {
    let env = base_env();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (client, _token_addr) = deploy_funded_stream(&env, &sender, &recipient, 100, 3600, false);

    // Depth starts at 0
    assert_eq!(guard_depth(&env, &client.address), 0);

    // Set depth to 1 (simulating one level of lock)
    env.as_contract(&client.address, || {
        env.storage()
            .instance()
            .set(&drip_stream::storage::DataKey::Guard, &1_u32);
    });

    // Should still fail since MAX_REENTRANCY_DEPTH is 1
    let result = client.try_withdraw(&1);
    assert_eq!(result, Err(Ok(Error::ReentrancyForbidden)));

    // But setting to 0 should work
    env.as_contract(&client.address, || {
        env.storage()
            .instance()
            .set(&drip_stream::storage::DataKey::Guard, &0_u32);
    });

    env.ledger().set(LedgerInfo {
        timestamp: env.ledger().timestamp() + 100,
        ..env.ledger().get()
    });
    client.withdraw(&1);
    assert_eq!(guard_depth(&env, &client.address), 0);
}

// ── Guard persistence across multiple calls ─────────────────────────────────

#[test]
fn test_guard_resets_between_independent_calls() {
    let env = base_env();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (client, _token_addr) = deploy_funded_stream(&env, &sender, &recipient, 100, 3600, false);

    // Perform many calls in sequence — the guard should reset each time
    for i in 1..=50 {
        env.ledger().set(LedgerInfo {
            timestamp: env.ledger().timestamp() + 10,
            ..env.ledger().get()
        });
        let withdrawn = client.withdraw(&(10 * i));
        assert!(withdrawn > 0);
        assert_eq!(guard_depth(&env, &client.address), 0);
    }
}

// ── Mathematical precision under stress ─────────────────────────────────────

#[test]
fn test_mathematical_precision_under_stress() {
    let env = base_env();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let tok_admin = token::StellarAssetClient::new(&env, &token_addr);

    // Use a very small rate and large duration to test precision at the limits
    let rate: i128 = 1;
    let duration: u64 = 100_000;
    let deposit = rate * duration as i128;
    tok_admin.mint(&sender, &deposit);

    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);

    token::Client::new(&env, &token_addr).transfer(&sender, &stream_id, &deposit);

    let now = env.ledger().timestamp();
    client.initialize(
        &sender,
        &recipient,
        &token_addr,
        &rate,
        &now,
        &(now + duration),
        &false,
        &2_592_000_u64,
    );

    // Perform many small withdrawals — each must release the guard
    for i in 1..=100 {
        env.ledger().set_timestamp(now + i);
        let withdrawable = client.withdrawable();
        assert!(withdrawable >= 1);
        client.withdraw(&1);
        assert_eq!(guard_depth(&env, &client.address), 0);
    }

    let info = client.info();
    assert_eq!(info.withdrawn, 100);
}

// ── Error handling across all operations ────────────────────────────────────

#[test]
fn test_guard_released_on_invalid_amount() {
    let env = base_env();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (client, _token_addr) = deploy_funded_stream(&env, &sender, &recipient, 100, 3600, false);

    // Try to withdraw a negative amount — should fail but release guard
    assert_eq!(guard_depth(&env, &client.address), 0);
    let result = client.try_withdraw(&-1);
    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
    assert_eq!(guard_depth(&env, &client.address), 0);
}

#[test]
fn test_guard_released_on_nothing_to_withdraw() {
    let env = base_env();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (client, _token_addr) = deploy_funded_stream(&env, &sender, &recipient, 100, 3600, false);

    // At t=0, nothing is withdrawable
    assert_eq!(guard_depth(&env, &client.address), 0);
    let result = client.try_withdraw(&1);
    assert_eq!(result, Err(Ok(Error::NothingToWithdraw)));
    assert_eq!(guard_depth(&env, &client.address), 0);
}

#[test]
fn test_guard_released_on_cancelled_stream() {
    let env = base_env();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (client, _token_addr) = deploy_funded_stream(&env, &sender, &recipient, 100, 3600, false);

    client.cancel(&sender);
    assert_eq!(guard_depth(&env, &client.address), 0);

    // Any further mutation on a cancelled stream fails but releases guard
    let result = client.try_withdraw(&1);
    assert_eq!(result, Err(Ok(Error::StreamCancelled)));
    assert_eq!(guard_depth(&env, &client.address), 0);

    let result = client.try_cancel(&sender);
    assert_eq!(result, Err(Ok(Error::StreamCancelled)));
    assert_eq!(guard_depth(&env, &client.address), 0);
}

#[test]
fn test_guard_released_on_already_paused() {
    let env = base_env();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (client, _token_addr) = deploy_funded_stream(&env, &sender, &recipient, 100, 3600, false);

    client.pause(&sender);
    assert_eq!(guard_depth(&env, &client.address), 0);

    let result = client.try_pause(&sender);
    assert_eq!(result, Err(Ok(Error::AlreadyPaused)));
    assert_eq!(guard_depth(&env, &client.address), 0);
}

#[test]
fn test_guard_released_on_not_paused() {
    let env = base_env();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (client, _token_addr) = deploy_funded_stream(&env, &sender, &recipient, 100, 3600, false);

    let result = client.try_resume(&sender);
    assert_eq!(result, Err(Ok(Error::NotPaused)));
    assert_eq!(guard_depth(&env, &client.address), 0);
}

#[test]
fn test_guard_released_on_clawback_disabled() {
    let env = base_env();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (client, _token_addr) = deploy_funded_stream(&env, &sender, &recipient, 100, 3600, false);

    let result = client.try_clawback(&sender);
    assert_eq!(result, Err(Ok(Error::ClawbackDisabled)));
    assert_eq!(guard_depth(&env, &client.address), 0);
}

#[test]
fn test_guard_released_on_pause_threshold_not_met() {
    let env = base_env();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (client, _token_addr) = deploy_funded_stream(&env, &sender, &recipient, 100, 3600, false);

    client.pause(&sender);

    // Time hasn't passed enough for force-cancel
    let result = client.try_force_cancel();
    assert_eq!(result, Err(Ok(Error::PauseThresholdNotMet)));
    assert_eq!(guard_depth(&env, &client.address), 0);
}

// ── Extend duration guard behavior ──────────────────────────────────────────

#[test]
fn test_guard_released_on_extend_duration_zero() {
    let env = base_env();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (client, _token_addr) = deploy_funded_stream(&env, &sender, &recipient, 100, 3600, false);

    let result = client.try_extend_duration(&sender, &0);
    assert_eq!(result, Err(Ok(Error::InvalidTimeRange)));
    assert_eq!(guard_depth(&env, &client.address), 0);
}

// ── Multiple operations in sequence ─────────────────────────────────────────

#[test]
fn test_sequential_operations_all_release_guard() {
    let env = base_env();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let new_recipient = Address::generate(&env);
    let (client, token_addr) = deploy_funded_stream(&env, &sender, &recipient, 100, 36000, true);

    let tok_admin = token::StellarAssetClient::new(&env, &token_addr);
    tok_admin.mint(&sender, &500_000);

    // Each operation releases the guard
    env.ledger().set_timestamp(env.ledger().timestamp() + 100);
    client.withdraw(&50);
    assert_eq!(guard_depth(&env, &client.address), 0);

    client.top_up(&sender, &100_000);
    assert_eq!(guard_depth(&env, &client.address), 0);

    client.pause(&sender);
    assert_eq!(guard_depth(&env, &client.address), 0);

    client.resume(&sender);
    assert_eq!(guard_depth(&env, &client.address), 0);

    client.transfer_recipient(&new_recipient);
    assert_eq!(guard_depth(&env, &client.address), 0);

    client.clawback(&sender);
    assert_eq!(guard_depth(&env, &client.address), 0);

    client.extend_duration(&sender, &3600);
    assert_eq!(guard_depth(&env, &client.address), 0);

    client.cancel(&sender);
    assert_eq!(guard_depth(&env, &client.address), 0);
}
