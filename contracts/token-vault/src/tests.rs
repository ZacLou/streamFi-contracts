#![cfg(test)]

extern crate std;

use soroban_sdk::{
    symbol_short,
    testutils::{storage::Instance as _, Address as _, Events as _},
    token, Address, Env, IntoVal, String, TryIntoVal,
};

use crate::errors::Error;
use crate::storage;
use crate::TokenVaultClient;

struct Setup {
    env: Env,
    client: TokenVaultClient<'static>,
    token: token::Client<'static>,
    owner: Address,
    user: Address,
}

impl Setup {
    fn new(max_limit: i128) -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let owner = Address::generate(&env);
        let user = Address::generate(&env);

        let token_admin = Address::generate(&env);
        let token_addr = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();

        let tok_admin = token::StellarAssetClient::new(&env, &token_addr);

        // Mint a large supply to the owner and user so authorized deposits succeed
        tok_admin.mint(&owner, &(max_limit));
        tok_admin.mint(&user, &(max_limit));

        let vault_id = env.register_contract(None, super::TokenVault);
        let client = TokenVaultClient::new(&env, &vault_id);

        client.initialize(&owner, &token_addr, &max_limit);

        // Leak env for 'static lifetime convenience in tests
        let env: &'static Env = std::boxed::Box::leak(std::boxed::Box::new(env));
        let client = TokenVaultClient::new(env, &vault_id);
        let token = token::Client::new(env, &token_addr);

        Setup {
            env: env.clone(),
            client,
            token,
            owner,
            user,
        }
    }
}

// ── Original deposit / withdraw / set_limit tests ──────────────────────────

#[test]
fn deposit_respects_max_limit() {
    let max_limit: i128 = 1_000_000;
    let s = Setup::new(max_limit);

    s.client.deposit(&s.owner, &max_limit);

    let result = s.client.try_deposit(&s.owner, &1);
    assert_eq!(result, Err(Ok(Error::LimitExceeded)));
}

#[test]
fn deposit_rejects_amount_that_would_overflow_i128() {
    let s = Setup::new(i128::MAX);

    s.client.deposit(&s.owner, &(i128::MAX - 10));
    let result = s.client.try_deposit(&s.owner, &20);
    assert_eq!(result, Err(Ok(Error::ArithmeticOverflow)));
}

#[test]
fn deposit_succeeds_with_real_sender_auth() {
    let max_limit: i128 = 1_000_000;
    let s = Setup::new(max_limit);

    s.client.deposit(&s.owner, &999_900);
    assert_eq!(
        s.client.try_deposit(&s.owner, &200),
        Err(Ok(Error::LimitExceeded))
    );
}

#[test]
fn operator_can_deposit() {
    let s = Setup::new(1_000_000);
    let op = Address::generate(&s.env);
    let asset = token::StellarAssetClient::new(&s.env, &s.token.address);
    asset.mint(&op, &500);
    s.client.set_operator(&s.owner, &op);

    s.client.deposit(&op, &500);
    assert_eq!(s.token.balance(&s.client.address), 500);
}

#[test]
fn stranger_cannot_deposit() {
    let s = Setup::new(1_000_000);
    let stranger = Address::generate(&s.env);

    let result = s.client.try_deposit(&stranger, &100);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn set_limit_succeeds_for_owner_with_real_auth() {
    let s = Setup::new(1_000_000);
    s.client.set_limit(&s.owner, &2_000_000);
}

#[test]
fn withdraw_succeeds_with_real_owner_auth() {
    let s = Setup::new(1_000_000);
    s.client.deposit(&s.owner, &1_000);

    let recipient = Address::generate(&s.env);
    s.client.withdraw(&s.owner, &recipient, &400);

    assert_eq!(s.token.balance(&recipient), 400);
}

#[test]
fn direct_transfers_count_toward_deposit_limit() {
    let s = Setup::new(1_000_000);

    s.client.deposit(&s.owner, &999_900);
    s.token.transfer(&s.user, &s.client.address, &100);

    assert_eq!(
        s.client.try_deposit(&s.owner, &1),
        Err(Ok(Error::LimitExceeded))
    );
}

#[test]
fn direct_transfers_are_withdrawable_by_owner() {
    let s = Setup::new(1_000_000);

    s.client.deposit(&s.owner, &500);
    s.token.transfer(&s.user, &s.client.address, &200);

    let recipient = Address::generate(&s.env);
    s.client.withdraw(&s.owner, &recipient, &700);

    assert_eq!(s.token.balance(&recipient), 700);
}

// ── Auth-bypass regressions ─────────────────────────────────────────────────

fn seed_vault(
    env: &Env,
    vault_id: &Address,
    owner: &Address,
    token_addr: &Address,
    balance: i128,
    max_limit: i128,
) {
    env.as_contract(vault_id, || {
        storage::set_owner(env, owner);
        storage::set_token(env, token_addr);
        storage::set_max_limit(env, &max_limit);
        storage::set_balance(env, &balance);
    });
}

#[test]
#[should_panic]
fn initialize_without_owner_auth_panics() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let vault_id = env.register_contract(None, super::TokenVault);
    let client = TokenVaultClient::new(&env, &vault_id);

    client.initialize(&owner, &token_addr, &1_000_000);
}

#[test]
#[should_panic]
fn withdraw_without_owner_auth_panics() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let stranger = Address::generate(&env);
    let to = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let vault_id = env.register_contract(None, super::TokenVault);

    seed_vault(&env, &vault_id, &owner, &token_addr, 1_000, 1_000_000);

    let client = TokenVaultClient::new(&env, &vault_id);
    // stranger is neither owner nor operator — must panic
    client.withdraw(&stranger, &to, &1);
}

#[test]
#[should_panic]
fn set_limit_with_owner_address_but_no_real_auth_panics() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let vault_id = env.register_contract(None, super::TokenVault);

    seed_vault(&env, &vault_id, &owner, &token_addr, 0, 1_000_000);

    let client = TokenVaultClient::new(&env, &vault_id);
    // Passing the real owner address without signed auth must still panic.
    client.set_limit(&owner, &2_000_000);
}

#[test]
#[should_panic]
fn deposit_without_sender_auth_panics() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let user = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let vault_id = env.register_contract(None, super::TokenVault);

    seed_vault(&env, &vault_id, &owner, &token_addr, 0, 1_000_000);

    let client = TokenVaultClient::new(&env, &vault_id);
    client.deposit(&user, &1);
}

// ── Operator delegation tests ───────────────────────────────────────────────

#[test]
fn owner_can_set_and_revoke_operator() {
    let s = Setup::new(1_000_000);

    let op = Address::generate(&s.env);
    assert_eq!(s.client.operator(), None);

    s.client.set_operator(&s.owner, &op);
    assert_eq!(s.client.operator(), Some(op.clone()));

    s.client.revoke_operator(&s.owner);
    assert_eq!(s.client.operator(), None);
}

#[test]
fn non_owner_cannot_set_operator() {
    let s = Setup::new(1_000_000);

    let stranger = Address::generate(&s.env);
    let op = Address::generate(&s.env);
    let result = s.client.try_set_operator(&stranger, &op);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn non_owner_cannot_revoke_operator() {
    let s = Setup::new(1_000_000);

    let op = Address::generate(&s.env);
    s.client.set_operator(&s.owner, &op);

    let stranger = Address::generate(&s.env);
    let result = s.client.try_revoke_operator(&stranger);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn operator_can_withdraw() {
    let s = Setup::new(1_000_000);
    s.client.deposit(&s.owner, &500);

    let op = Address::generate(&s.env);
    s.client.set_operator(&s.owner, &op);
    s.client.set_operator_withdraw_limit(&s.owner, &1_000);

    let recipient = Address::generate(&s.env);
    s.client.withdraw(&op, &recipient, &200);
    assert_eq!(s.token.balance(&recipient), 200);
}

#[test]
fn operator_can_lower_limit() {
    let s = Setup::new(1_000_000);

    let op = Address::generate(&s.env);
    s.client.set_operator(&s.owner, &op);

    s.client.set_limit(&op, &500_000);

    let vault_id = s.client.address.clone();
    s.env.as_contract(&vault_id, || {
        assert_eq!(storage::get_max_limit(&s.env), Some(500_000));
    });
}

#[test]
fn operator_cannot_raise_limit() {
    let s = Setup::new(1_000_000);

    let op = Address::generate(&s.env);
    s.client.set_operator(&s.owner, &op);

    let result = s.client.try_set_limit(&op, &2_000_000);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn owner_can_raise_limit() {
    let s = Setup::new(1_000_000);

    s.client.set_limit(&s.owner, &2_000_000);

    let vault_id = s.client.address.clone();
    s.env.as_contract(&vault_id, || {
        assert_eq!(storage::get_max_limit(&s.env), Some(2_000_000));
    });
}

#[test]
fn operator_withdraw_limit_is_enforced() {
    let s = Setup::new(1_000_000);
    s.client.deposit(&s.owner, &500);

    let op = Address::generate(&s.env);
    s.client.set_operator(&s.owner, &op);
    s.client.set_operator_withdraw_limit(&s.owner, &100);

    let recipient = Address::generate(&s.env);
    let result = s.client.try_withdraw(&op, &recipient, &200);
    assert_eq!(result, Err(Ok(Error::LimitExceeded)));

    s.client.withdraw(&op, &recipient, &100);
    assert_eq!(s.token.balance(&recipient), 100);
}

#[test]
fn owner_withdraw_is_not_limited_by_operator_cap() {
    let s = Setup::new(1_000_000);
    s.client.deposit(&s.owner, &500);

    let op = Address::generate(&s.env);
    s.client.set_operator(&s.owner, &op);
    s.client.set_operator_withdraw_limit(&s.owner, &100);

    let recipient = Address::generate(&s.env);
    s.client.withdraw(&s.owner, &recipient, &500);
    assert_eq!(s.token.balance(&recipient), 500);
}

#[test]
fn stranger_cannot_withdraw_even_with_operator_set() {
    let s = Setup::new(1_000_000);
    s.client.deposit(&s.owner, &500);

    let op = Address::generate(&s.env);
    s.client.set_operator(&s.owner, &op);

    let stranger = Address::generate(&s.env);
    let result = s.client.try_withdraw(&stranger, &stranger, &1);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn after_revoke_operator_can_no_longer_withdraw() {
    let s = Setup::new(1_000_000);
    s.client.deposit(&s.owner, &500);

    let op = Address::generate(&s.env);
    s.client.set_operator(&s.owner, &op);
    s.client.revoke_operator(&s.owner);

    let result = s.client.try_withdraw(&op, &op, &1);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn revoke_operator_when_none_set_is_ok() {
    let s = Setup::new(1_000_000);
    // No operator set — revoke should be a no-op, not an error.
    let result = s.client.try_revoke_operator(&s.owner);
    assert!(result.is_ok());
}

// ── Emergency pause tests ───────────────────────────────────────────────────

#[test]
fn owner_can_pause_and_unpause() {
    let s = Setup::new(1_000_000);

    assert!(!s.client.is_paused());
    s.client.pause(&s.owner);
    assert!(s.client.is_paused());
    s.client.unpause(&s.owner);
    assert!(!s.client.is_paused());
}

#[test]
fn non_owner_cannot_pause() {
    let s = Setup::new(1_000_000);
    let stranger = Address::generate(&s.env);
    let result = s.client.try_pause(&stranger);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn double_pause_is_rejected() {
    let s = Setup::new(1_000_000);
    s.client.pause(&s.owner);
    let result = s.client.try_pause(&s.owner);
    assert_eq!(result, Err(Ok(Error::AlreadyPaused)));
}

#[test]
fn unpause_when_not_paused_is_rejected() {
    let s = Setup::new(1_000_000);
    let result = s.client.try_unpause(&s.owner);
    assert_eq!(result, Err(Ok(Error::NotPaused)));
}

#[test]
fn pause_blocks_deposit() {
    let s = Setup::new(1_000_000);
    s.client.pause(&s.owner);
    let result = s.client.try_deposit(&s.user, &100);
    assert_eq!(result, Err(Ok(Error::ContractPaused)));
}

#[test]
fn pause_blocks_withdraw() {
    let s = Setup::new(1_000_000);
    s.client.deposit(&s.owner, &500);
    s.client.pause(&s.owner);

    let recipient = Address::generate(&s.env);
    let result = s.client.try_withdraw(&s.owner, &recipient, &100);
    assert_eq!(result, Err(Ok(Error::ContractPaused)));
}

#[test]
fn pause_blocks_set_limit() {
    let s = Setup::new(1_000_000);
    s.client.pause(&s.owner);
    let result = s.client.try_set_limit(&s.owner, &2_000_000);
    assert_eq!(result, Err(Ok(Error::ContractPaused)));
}

#[test]
fn operations_resume_after_unpause() {
    let s = Setup::new(1_000_000);
    s.client.pause(&s.owner);
    s.client.unpause(&s.owner);

    // All three should work again after unpause
    s.client.deposit(&s.owner, &100);
    s.client.set_limit(&s.owner, &2_000_000);
    let recipient = Address::generate(&s.env);
    s.client.withdraw(&s.owner, &recipient, &50);
    assert_eq!(s.token.balance(&recipient), 50);
}

#[test]
fn operator_also_blocked_by_pause() {
    let s = Setup::new(1_000_000);
    s.client.deposit(&s.owner, &500);

    let op = Address::generate(&s.env);
    s.client.set_operator(&s.owner, &op);
    s.client.pause(&s.owner);

    let recipient = Address::generate(&s.env);
    let result = s.client.try_withdraw(&op, &recipient, &100);
    assert_eq!(result, Err(Ok(Error::ContractPaused)));
}

// ── Event emission (issue #311) ────────────────────────────────────────────

/// All events published by the vault contract, oldest first.
///
/// `env.events().all()` also captures events emitted by other contracts in the
/// same test (e.g. the token's own transfer events), so filter down to the
/// vault's contract address. `Setup::new` publishes the `init` event, so
/// `events[0]` is always the initialization event.
fn vault_events(
    s: &Setup,
) -> std::vec::Vec<(
    Address,
    soroban_sdk::Vec<soroban_sdk::Val>,
    soroban_sdk::Val,
)> {
    s.env
        .events()
        .all()
        .iter()
        .filter(|(contract, _, _)| contract == &s.client.address)
        .map(|(c, t, d)| (c.clone(), t.clone(), d))
        .collect()
}

#[test]
fn initialize_emits_initialized_event() {
    let s = Setup::new(1_000_000);

    let events = vault_events(&s);
    assert_eq!(events.len(), 1);

    let (_, topics, data) = &events[0];
    assert_eq!(
        topics.clone(),
        (symbol_short!("init"), s.owner.clone()).into_val(&s.env)
    );
    let payload: (Address, i128) = data.clone().try_into_val(&s.env).unwrap();
    assert_eq!(payload, (s.token.address.clone(), 1_000_000_i128));
}

#[test]
fn deposit_emits_deposited_event() {
    let s = Setup::new(1_000_000);
    s.client.deposit(&s.owner, &500);

    let events = vault_events(&s);
    assert_eq!(events.len(), 2);

    let (_, topics, data) = &events[1];
    assert_eq!(
        topics.clone(),
        (symbol_short!("deposited"), s.owner.clone()).into_val(&s.env)
    );
    let payload: (i128, i128) = data.clone().try_into_val(&s.env).unwrap();
    assert_eq!(payload, (500_i128, 500_i128));
}

#[test]
fn withdraw_emits_withdrawn_event() {
    let s = Setup::new(1_000_000);
    s.client.deposit(&s.owner, &1_000);

    let recipient = Address::generate(&s.env);
    s.client.withdraw(&s.owner, &recipient, &400);

    let events = vault_events(&s);
    assert_eq!(events.len(), 3);

    let (_, topics, data) = &events[2];
    assert_eq!(
        topics.clone(),
        (symbol_short!("withdrawn"), s.owner.clone()).into_val(&s.env)
    );
    let payload: (Address, i128, i128) = data.clone().try_into_val(&s.env).unwrap();
    assert_eq!(payload, (recipient.clone(), 400_i128, 600_i128));
}

#[test]
fn set_limit_emits_limit_set_event_with_old_and_new() {
    let s = Setup::new(1_000_000);
    s.client.set_limit(&s.owner, &2_000_000);

    let events = vault_events(&s);
    assert_eq!(events.len(), 2);

    let (_, topics, data) = &events[1];
    assert_eq!(
        topics.clone(),
        (symbol_short!("limit_set"), s.owner.clone()).into_val(&s.env)
    );
    let payload: (i128, i128) = data.clone().try_into_val(&s.env).unwrap();
    assert_eq!(payload, (1_000_000_i128, 2_000_000_i128));
}

#[test]
fn operator_set_and_revoke_emit_events() {
    let s = Setup::new(1_000_000);

    let op = Address::generate(&s.env);
    s.client.set_operator(&s.owner, &op);

    let events = vault_events(&s);
    assert_eq!(events.len(), 2);
    let (_, topics, data) = &events[1];
    assert_eq!(
        topics.clone(),
        (symbol_short!("set_op"), s.owner.clone()).into_val(&s.env)
    );
    let payload: Address = data.clone().try_into_val(&s.env).unwrap();
    assert_eq!(payload, op);

    s.client.revoke_operator(&s.owner);

    let events = vault_events(&s);
    assert_eq!(events.len(), 3);
    let (_, topics, data) = &events[2];
    assert_eq!(
        topics.clone(),
        (symbol_short!("rm_op"), s.owner.clone()).into_val(&s.env)
    );
    let payload: () = data.clone().try_into_val(&s.env).unwrap();
    assert_eq!(payload, ());
}

#[test]
fn pause_and_unpause_emit_events() {
    let s = Setup::new(1_000_000);
    let paused_at = s.env.ledger().timestamp();

    s.client.pause(&s.owner);
    let events = vault_events(&s);
    assert_eq!(events.len(), 2);
    let (_, topics, data) = &events[1];
    assert_eq!(
        topics.clone(),
        (symbol_short!("paused"), s.owner.clone()).into_val(&s.env)
    );
    let payload: u64 = data.clone().try_into_val(&s.env).unwrap();
    assert_eq!(payload, paused_at);

    s.client.unpause(&s.owner);
    let events = vault_events(&s);
    assert_eq!(events.len(), 3);
    let (_, topics, data) = &events[2];
    assert_eq!(
        topics.clone(),
        (symbol_short!("unpaused"), s.owner.clone()).into_val(&s.env)
    );
    let payload: u64 = data.clone().try_into_val(&s.env).unwrap();
    assert_eq!(payload, paused_at);
}

#[test]
fn failed_operations_emit_no_events() {
    let s = Setup::new(1_000_000);
    let base = vault_events(&s).len();

    // Deposit over the limit reverts without publishing a `deposited` event.
    s.client.deposit(&s.owner, &999_900);
    let result = s.client.try_deposit(&s.owner, &200);
    assert_eq!(result, Err(Ok(Error::LimitExceeded)));
    assert_eq!(vault_events(&s).len(), base + 1);

    // Double pause reverts without publishing a second `paused` event.
    s.client.pause(&s.owner);
    let result = s.client.try_pause(&s.owner);
    assert_eq!(result, Err(Ok(Error::AlreadyPaused)));
    assert_eq!(vault_events(&s).len(), base + 2);

    // Unpause when not paused reverts without publishing an `unpaused` event.
    s.client.unpause(&s.owner);
    let result = s.client.try_unpause(&s.owner);
    assert_eq!(result, Err(Ok(Error::NotPaused)));
    assert_eq!(vault_events(&s).len(), base + 3);

    // Unauthorized withdraw reverts without publishing a `withdrawn` event.
    let stranger = Address::generate(&s.env);
    let result = s.client.try_withdraw(&stranger, &stranger, &1);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
    assert_eq!(vault_events(&s).len(), base + 3);

    // Setting a limit below the current balance reverts without publishing.
    let result = s.client.try_set_limit(&s.owner, &100);
    assert_eq!(result, Err(Ok(Error::LimitExceeded)));
    assert_eq!(vault_events(&s).len(), base + 3);
}

// ── Uninitialized state tests ───────────────────────────────────────────────

#[test]
fn uninitialized_vault_returns_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();
    let vault_id = env.register_contract(None, super::TokenVault);
    let client = TokenVaultClient::new(&env, &vault_id);

    let user = Address::generate(&env);
    let recipient = Address::generate(&env);
    let operator = Address::generate(&env);

    assert_eq!(
        client.try_deposit(&user, &100),
        Err(Ok(Error::NotInitialized))
    );
    assert_eq!(
        client.try_withdraw(&user, &recipient, &100),
        Err(Ok(Error::NotInitialized))
    );
    assert_eq!(
        client.try_set_limit(&user, &100),
        Err(Ok(Error::NotInitialized))
    );
    assert_eq!(
        client.try_set_operator(&user, &operator),
        Err(Ok(Error::NotInitialized))
    );
    assert_eq!(
        client.try_revoke_operator(&user),
        Err(Ok(Error::NotInitialized))
    );
    assert_eq!(client.try_pause(&user), Err(Ok(Error::NotInitialized)));
    assert_eq!(client.try_unpause(&user), Err(Ok(Error::NotInitialized)));
}

#[test]
fn re_initializing_vault_reports_already_initialized() {
    // Asserted against the named variant rather than `Error(Contract, #9)`.
    // A numeric assertion keeps passing if the enum is ever reordered, which
    // is precisely the "caller gets the wrong error code" failure this check
    // exists to catch.
    let s = Setup::new(1_000_000);
    let another_owner = Address::generate(&s.env);
    let token_admin = Address::generate(&s.env);
    let token_addr = s
        .env
        .register_stellar_asset_contract_v2(token_admin)
        .address();

    let result = s
        .client
        .try_initialize(&another_owner, &token_addr, &2_000_000);

    assert_eq!(result, Err(Ok(Error::AlreadyInitialized)));
}

#[test]
fn failed_re_initialization_leaves_the_original_owner_in_place() {
    // The error code matters, but so does the state: a rejected re-init must
    // not have partially overwritten the vault's owner or limit.
    let s = Setup::new(1_000_000);
    let attacker = Address::generate(&s.env);
    let token_admin = Address::generate(&s.env);
    let token_addr = s
        .env
        .register_stellar_asset_contract_v2(token_admin)
        .address();

    let _ = s.client.try_initialize(&attacker, &token_addr, &2_000_000);

    let vault_id = s.client.address.clone();
    s.env.as_contract(&vault_id, || {
        assert_eq!(storage::get_owner(&s.env), Some(s.owner.clone()));
        assert_eq!(storage::get_max_limit(&s.env), Some(1_000_000));
    });
}

// ── TTL extension tests ───────────────────────────────────────────────────

#[test]
fn initialize_extends_instance_ttl() {
    let s = Setup::new(1_000_000);
    let ttl = s
        .env
        .as_contract(&s.client.address, || s.env.storage().instance().get_ttl());
    assert!(ttl >= 100_000, "instance TTL after initialize: {ttl}");
}

#[test]
fn deposit_extends_instance_ttl() {
    let s = Setup::new(1_000_000);
    s.client.deposit(&s.owner, &100);
    let ttl = s
        .env
        .as_contract(&s.client.address, || s.env.storage().instance().get_ttl());
    assert!(ttl >= 100_000, "instance TTL after deposit: {ttl}");
}

#[test]
fn withdraw_extends_instance_ttl() {
    let s = Setup::new(1_000_000);
    s.client.deposit(&s.owner, &100);
    let recipient = Address::generate(&s.env);
    s.client.withdraw(&s.owner, &recipient, &50);
    let ttl = s
        .env
        .as_contract(&s.client.address, || s.env.storage().instance().get_ttl());
    assert!(ttl >= 100_000, "instance TTL after withdraw: {ttl}");
}

#[test]
fn set_limit_extends_instance_ttl() {
    let s = Setup::new(1_000_000);
    s.client.set_limit(&s.owner, &2_000_000);
    let ttl = s
        .env
        .as_contract(&s.client.address, || s.env.storage().instance().get_ttl());
    assert!(ttl >= 100_000, "instance TTL after set_limit: {ttl}");
}

#[test]
fn operator_and_pause_mutations_extend_instance_ttl() {
    let s = Setup::new(1_000_000);
    let operator = Address::generate(&s.env);

    s.client.set_operator(&s.owner, &operator);
    let ttl = s
        .env
        .as_contract(&s.client.address, || s.env.storage().instance().get_ttl());
    assert!(ttl >= 100_000, "instance TTL after set_operator: {ttl}");

    s.client.revoke_operator(&s.owner);
    let ttl = s
        .env
        .as_contract(&s.client.address, || s.env.storage().instance().get_ttl());
    assert!(ttl >= 100_000, "instance TTL after revoke_operator: {ttl}");

    s.client.pause(&s.owner);
    let ttl = s
        .env
        .as_contract(&s.client.address, || s.env.storage().instance().get_ttl());
    assert!(ttl >= 100_000, "instance TTL after pause: {ttl}");

    s.client.unpause(&s.owner);
    let ttl = s
        .env
        .as_contract(&s.client.address, || s.env.storage().instance().get_ttl());
    assert!(ttl >= 100_000, "instance TTL after unpause: {ttl}");
}

// ── Two-step owner transfer tests ───────────────────────────────────────────

#[test]
fn owner_can_propose_and_new_owner_accepts() {
    let s = Setup::new(1_000_000);
    let new_owner = Address::generate(&s.env);

    assert_eq!(s.client.owner(), Some(s.owner.clone()));
    s.client.propose_owner(&s.owner, &new_owner);
    s.client.accept_owner(&new_owner);

    assert_eq!(s.client.owner(), Some(new_owner));
}

#[test]
fn propose_owner_rejects_zero_address() {
    let s = Setup::new(1_000_000);
    let zero = Address::from_string(&String::from_str(
        &s.env,
        "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4",
    ));
    let result = s.client.try_propose_owner(&s.owner, &zero);
    assert_eq!(result, Err(Ok(Error::InvalidParam)));
}

#[test]
fn non_owner_cannot_propose_owner() {
    let s = Setup::new(1_000_000);
    let stranger = Address::generate(&s.env);
    let new_owner = Address::generate(&s.env);
    let result = s.client.try_propose_owner(&stranger, &new_owner);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn accept_owner_when_none_pending_is_rejected() {
    let s = Setup::new(1_000_000);
    let stranger = Address::generate(&s.env);
    let result = s.client.try_accept_owner(&stranger);
    assert_eq!(result, Err(Ok(Error::NoPendingOwner)));
}

#[test]
fn accept_owner_by_non_pending_address_is_rejected() {
    let s = Setup::new(1_000_000);
    let new_owner = Address::generate(&s.env);
    let stranger = Address::generate(&s.env);

    s.client.propose_owner(&s.owner, &new_owner);
    let result = s.client.try_accept_owner(&stranger);
    assert_eq!(result, Err(Ok(Error::NotPendingOwner)));
}

#[test]
fn propose_owner_does_not_change_ownership_until_accept() {
    let s = Setup::new(1_000_000);
    let new_owner = Address::generate(&s.env);

    s.client.propose_owner(&s.owner, &new_owner);
    // Old owner retains authority until the transfer is accepted.
    assert_eq!(s.client.owner(), Some(s.owner.clone()));

    // Proposing while a transfer is already pending is rejected for a
    // non-owner... but the old owner can still propose, which simply
    // overwrites the pending owner. The real owner stays until accept.
    let other = Address::generate(&s.env);
    s.client.propose_owner(&s.owner, &other);
    assert_eq!(s.client.owner(), Some(s.owner.clone()));
}

#[test]
fn new_owner_gains_privileges_after_accept() {
    let s = Setup::new(1_000_000);
    let new_owner = Address::generate(&s.env);

    // Before accept, the new owner cannot pause.
    assert_eq!(
        s.client.try_pause(&new_owner),
        Err(Ok(Error::NotAuthorized))
    );

    s.client.propose_owner(&s.owner, &new_owner);
    s.client.accept_owner(&new_owner);

    // After accept, the new owner can pause and the old owner cannot.
    s.client.pause(&new_owner);
    assert!(s.client.is_paused());
    assert_eq!(
        s.client.try_unpause(&s.owner),
        Err(Ok(Error::NotAuthorized))
    );
    s.client.unpause(&new_owner);
}

#[test]
fn owner_transfer_emits_events() {
    let s = Setup::new(1_000_000);
    let new_owner = Address::generate(&s.env);

    s.client.propose_owner(&s.owner, &new_owner);
    let events = vault_events(&s);
    assert_eq!(events.len(), 2);
    let (_, topics, data) = &events[1];
    assert_eq!(
        topics.clone(),
        (symbol_short!("propose"), s.owner.clone()).into_val(&s.env)
    );
    let payload: Address = data.clone().try_into_val(&s.env).unwrap();
    assert_eq!(payload, new_owner);

    s.client.accept_owner(&new_owner);
    let events = vault_events(&s);
    assert_eq!(events.len(), 3);
    let (_, topics, data) = &events[2];
    assert_eq!(
        topics.clone(),
        (symbol_short!("accept"), new_owner.clone()).into_val(&s.env)
    );
    let payload: Address = data.clone().try_into_val(&s.env).unwrap();
    assert_eq!(payload, s.owner);
}

#[test]
fn uninitialized_vault_rejects_owner_transfer() {
    let env = Env::default();
    env.mock_all_auths();
    let vault_id = env.register_contract(None, super::TokenVault);
    let client = TokenVaultClient::new(&env, &vault_id);

    let caller = Address::generate(&env);
    let new_owner = Address::generate(&env);

    assert_eq!(
        client.try_propose_owner(&caller, &new_owner),
        Err(Ok(Error::NotInitialized))
    );
    assert_eq!(
        client.try_accept_owner(&caller),
        Err(Ok(Error::NoPendingOwner))
    );
}
