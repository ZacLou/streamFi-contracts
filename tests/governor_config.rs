//! Integration tests: DripGovernor parameter management.

use drip_governor::{DripGovernor, DripGovernorClient, Error};
use soroban_sdk::{
    testutils::{storage::Instance as _, Address as _, Events},
    Address, Env,
};

fn deploy_governor(env: &Env) -> (DripGovernorClient<'_>, Address, Address) {
    let authority = Address::generate(env);
    let fee_recipient = Address::generate(env);
    let factory_address = Address::generate(env);

    let id = env.register_contract(None, DripGovernor);
    let client = DripGovernorClient::new(env, &id);

    client.initialize(&authority, &fee_recipient, &factory_address);

    (client, authority, fee_recipient)
}

// ── Defaults ─────────────────────────────────────────────────────────────────

#[test]
fn initialize_sets_correct_defaults() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _authority, fee_recipient) = deploy_governor(&env);
    let config = client.config();

    assert_eq!(config.fee_bps, 30);
    assert_eq!(config.min_duration_seconds, 3_600);
    assert_eq!(config.max_rate_per_second, 1_000_000_000_000_000);
    assert_eq!(config.fee_recipient, fee_recipient);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn re_initializing_governor_panics() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _authority, _fee_recipient) = deploy_governor(&env);
    // An attacker calling initialize() again to grant themselves Admin must be
    // rejected — otherwise they could set fee_bps to the maximum or redirect
    // fee_recipient.
    let attacker = Address::generate(&env);
    client.initialize(&attacker, &attacker, &attacker);
}

// ── TTL management ─────────────────────────────────────────────────────────────

#[test]
fn initialize_extends_instance_ttl() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _authority, _fee_recipient) = deploy_governor(&env);
    let ttl = env.as_contract(&client.address, || env.storage().instance().get_ttl());
    assert_eq!(ttl, 200_000);
}

#[test]
fn set_fee_bps_extends_instance_ttl() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _fee_recipient) = deploy_governor(&env);
    client.set_fee_bps(&authority, &50);
    let ttl = env.as_contract(&client.address, || env.storage().instance().get_ttl());
    assert_eq!(ttl, 200_000);
}

// ── Fee BPS ──────────────────────────────────────────────────────────────────

#[test]
fn authority_can_update_fee_bps() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    client.set_fee_bps(&authority, &50);
    assert_eq!(client.config().fee_bps, 50);
}

#[test]
fn administrative_updates_emit_events() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    client.set_fee_bps(&authority, &50);

    let events = env.events().all();
    assert!(!events.is_empty());
}

#[test]
fn fee_bps_of_zero_is_valid() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    client.set_fee_bps(&authority, &0);
    assert_eq!(client.config().fee_bps, 0);
}

#[test]
fn fee_bps_of_10000_is_valid() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    client.set_fee_bps(&authority, &10_000);
    assert_eq!(client.config().fee_bps, 10_000);
}

#[test]
fn fee_bps_exceeding_10000_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    let result = client.try_set_fee_bps(&authority, &10_001);
    assert_eq!(result, Err(Ok(Error::InvalidParam)));
}

// ── Min duration ─────────────────────────────────────────────────────────────

#[test]
fn authority_can_set_min_duration() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    client.set_min_duration(&authority, &7_200);
    assert_eq!(client.config().min_duration_seconds, 7_200);
}

#[test]
fn min_duration_view_matches_config() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    assert_eq!(client.min_duration(), 3_600);

    client.set_min_duration(&authority, &7_200);
    assert_eq!(client.min_duration(), client.config().min_duration_seconds);
    assert_eq!(client.min_duration(), 7_200);
}

/// `min_duration()` must error the same way `max_duration()`/`max_rate()`/
/// `config()` do for an uninitialised governor, rather than silently
/// returning a hardcoded default (#424) — otherwise a caller cannot tell
/// "governor says 3600" from "governor doesn't exist yet".
#[test]
fn min_duration_errors_when_uninitialized() {
    let env = Env::default();
    env.mock_all_auths();

    let id = env.register_contract(None, DripGovernor);
    let client = DripGovernorClient::new(&env, &id);

    assert_eq!(
        client.try_min_duration(),
        Err(Ok(Error::NotInitialized))
    );
    assert_eq!(
        client.try_min_duration().is_err(),
        client.try_max_duration().is_err()
    );
    assert_eq!(
        client.try_min_duration().is_err(),
        client.try_max_rate().is_err()
    );
}

#[test]
fn zero_min_duration_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    let result = client.try_set_min_duration(&authority, &0);
    assert_eq!(result, Err(Ok(Error::InvalidParam)));
}

#[test]
fn min_duration_exceeding_max_duration_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    let max_dur = client.config().max_duration_seconds;
    let result = client.try_set_min_duration(&authority, &(max_dur + 1));
    assert_eq!(result, Err(Ok(Error::InvalidParam)));
}

#[test]
fn max_duration_below_min_duration_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    let min_dur = client.config().min_duration_seconds;
    let result = client.try_set_max_duration(&authority, &(min_dur - 1));
    assert_eq!(result, Err(Ok(Error::InvalidParam)));
}

// ── Max rate ─────────────────────────────────────────────────────────────────

#[test]
fn authority_can_set_max_rate() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    client.set_max_rate(&authority, &500_000_000);
    assert_eq!(client.config().max_rate_per_second, 500_000_000);
}

#[test]
fn zero_max_rate_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    let result = client.try_set_max_rate(&authority, &0);
    assert_eq!(result, Err(Ok(Error::InvalidParam)));
}

#[test]
fn negative_max_rate_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    let result = client.try_set_max_rate(&authority, &-1);
    assert_eq!(result, Err(Ok(Error::InvalidParam)));
}

#[test]
fn max_rate_matches_config() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    assert_eq!(client.max_rate(), client.config().max_rate_per_second);

    client.set_max_rate(&authority, &500_000_000);
    assert_eq!(client.max_rate(), 500_000_000);
}

// ── Fee recipient ────────────────────────────────────────────────────────────

#[test]
fn authority_can_change_fee_recipient() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    let new_recipient = Address::generate(&env);
    client.set_fee_recipient(&authority, &new_recipient);
    assert_eq!(client.config().fee_recipient, new_recipient);
}

// ── Max duration ────────────────────────────────────────────────────────────

#[test]
fn authority_can_set_max_duration() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    client.set_max_duration(&authority, &7_200);
    assert_eq!(client.config().max_duration_seconds, 7_200);
}

#[test]
fn zero_max_duration_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    let result = client.try_set_max_duration(&authority, &0);
    assert_eq!(result, Err(Ok(Error::InvalidParam)));
}

#[test]
fn non_rate_manager_cannot_set_max_duration() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _authority, _) = deploy_governor(&env);
    let non_rate_manager = Address::generate(&env);
    let result = client.try_set_max_duration(&non_rate_manager, &7_200);
    assert!(result.is_err());
}

#[test]
fn max_duration_matches_config() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    assert_eq!(client.max_duration(), client.config().max_duration_seconds);

    client.set_max_duration(&authority, &7_200);
    assert_eq!(client.max_duration(), 7_200);
}

// ── Transfer authority ───────────────────────────────────────────────────────

#[test]
#[allow(deprecated)]
fn authority_transfers_correctly() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, old_authority, _) = deploy_governor(&env);
    let new_authority = Address::generate(&env);
    client.transfer_authority(&old_authority, &new_authority);

    // Post-transfer, a config read still works (roles are stored, not verified
    // on read).
    let config = client.config();
    assert_eq!(config.fee_bps, 30); // defaults unchanged
}

// ── TTL extension on all state-mutating functions ───────────────────────────

#[test]
fn grant_role_extends_instance_ttl() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    let new_admin = Address::generate(&env);
    client.grant_role(&authority, &drip_governor::Role::Admin, &new_admin);
    let ttl = env.as_contract(&client.address, || env.storage().instance().get_ttl());
    assert_eq!(ttl, 200_000);
}

#[test]
fn revoke_role_extends_instance_ttl() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    let new_admin = Address::generate(&env);
    client.grant_role(&authority, &drip_governor::Role::Admin, &new_admin);
    // Reset to baseline after grant
    env.as_contract(&client.address, || {
        env.storage().instance().extend_ttl(100_000, 200_000);
    });

    client.revoke_role(&authority, &drip_governor::Role::Admin, &new_admin);
    let ttl = env.as_contract(&client.address, || env.storage().instance().get_ttl());
    assert_eq!(ttl, 200_000);
}

#[test]
#[allow(deprecated)]
fn transfer_authority_extends_instance_ttl() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    let new_authority = Address::generate(&env);
    client.transfer_authority(&authority, &new_authority);
    let ttl = env.as_contract(&client.address, || env.storage().instance().get_ttl());
    assert_eq!(ttl, 200_000);
}

#[test]
fn set_fee_recipient_extends_instance_ttl() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    let new_recipient = Address::generate(&env);
    client.set_fee_recipient(&authority, &new_recipient);
    let ttl = env.as_contract(&client.address, || env.storage().instance().get_ttl());
    assert_eq!(ttl, 200_000);
}

#[test]
fn set_fee_recipient_rejects_zero_address() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, fee_recipient) = deploy_governor(&env);
    let zero_account = Address::from_string(&soroban_sdk::String::from_str(
        &env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    ));

    let result = client.try_set_fee_recipient(&authority, &zero_account);
    assert_eq!(result, Err(Ok(Error::InvalidParam)));

    // The rejected call must not have mutated state.
    assert_eq!(client.config().fee_recipient, fee_recipient);
}

#[test]
fn set_fee_recipient_rejects_zero_contract_address() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, fee_recipient) = deploy_governor(&env);
    let zero_contract = Address::from_string(&soroban_sdk::String::from_str(
        &env,
        "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4",
    ));

    let result = client.try_set_fee_recipient(&authority, &zero_contract);
    assert_eq!(result, Err(Ok(Error::InvalidParam)));

    // The rejected call must not have mutated state.
    assert_eq!(client.config().fee_recipient, fee_recipient);
}

#[test]
fn set_min_duration_extends_instance_ttl() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    client.set_min_duration(&authority, &7_200);
    let ttl = env.as_contract(&client.address, || env.storage().instance().get_ttl());
    assert_eq!(ttl, 200_000);
}

#[test]
fn set_max_rate_extends_instance_ttl() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    client.set_max_rate(&authority, &500_000_000);
    let ttl = env.as_contract(&client.address, || env.storage().instance().get_ttl());
    assert_eq!(ttl, 200_000);
}

#[test]
fn set_max_duration_extends_instance_ttl() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    client.set_max_duration(&authority, &7_200);
    let ttl = env.as_contract(&client.address, || env.storage().instance().get_ttl());
    assert_eq!(ttl, 200_000);
}

// ── Authorization checks for rate-manager-gated setters ──────────────────────

#[test]
fn non_rate_manager_cannot_set_min_duration() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _authority, _) = deploy_governor(&env);
    let non_rate_manager = Address::generate(&env);
    let result = client.try_set_min_duration(&non_rate_manager, &7_200);
    assert!(result.is_err());
}

#[test]
fn non_rate_manager_cannot_set_max_rate() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _authority, _) = deploy_governor(&env);
    let non_rate_manager = Address::generate(&env);
    let result = client.try_set_max_rate(&non_rate_manager, &500_000_000);
    assert!(result.is_err());
}

// ── Authorization checks for fee-manager-gated setters ──────────────────────

#[test]
fn non_fee_manager_cannot_set_fee_bps() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _authority, _) = deploy_governor(&env);
    let non_fee_manager = Address::generate(&env);
    let result = client.try_set_fee_bps(&non_fee_manager, &50);
    assert!(result.is_err());
}

#[test]
fn non_fee_manager_cannot_set_fee_recipient() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _authority, _) = deploy_governor(&env);
    let non_fee_manager = Address::generate(&env);
    let new_recipient = Address::generate(&env);
    let result = client.try_set_fee_recipient(&non_fee_manager, &new_recipient);
    assert!(result.is_err());
}

#[test]
fn set_fee_bps_validates_param_early() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _authority, _) = deploy_governor(&env);
    let unauthorized = Address::generate(&env);

    // Out-of-bounds fee (>10,000) fails early with InvalidParam
    let result = client.try_set_fee_bps(&unauthorized, &15_000);
    assert_eq!(result, Err(Ok(Error::InvalidParam)));
}

#[test]
fn propose_authority_rejects_zero_address_and_existing_admin() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    let zero_account = Address::from_string(&soroban_sdk::String::from_str(
        &env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    ));

    // Zero address proposal rejected
    let res_zero = client.try_propose_authority(&authority, &zero_account);
    assert_eq!(res_zero, Err(Ok(Error::InvalidParam)));

    // Existing admin proposal rejected
    let res_existing = client.try_propose_authority(&authority, &authority);
    assert_eq!(res_existing, Err(Ok(Error::InvalidParam)));
}

#[test]
fn propose_authority_rejects_zero_contract_address() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    let zero_contract = Address::from_string(&soroban_sdk::String::from_str(
        &env,
        "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4",
    ));

    let result = client.try_propose_authority(&authority, &zero_contract);
    assert_eq!(result, Err(Ok(Error::InvalidParam)));
}

#[test]
#[allow(deprecated)]
fn transfer_authority_rejects_zero_address_and_existing_admin() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    let zero_account = Address::from_string(&soroban_sdk::String::from_str(
        &env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    ));

    // Zero address transfer rejected
    let res_zero = client.try_transfer_authority(&authority, &zero_account);
    assert_eq!(res_zero, Err(Ok(Error::InvalidParam)));

    // Self/existing admin transfer rejected
    let res_existing = client.try_transfer_authority(&authority, &authority);
    assert_eq!(res_existing, Err(Ok(Error::InvalidParam)));
}

#[test]
#[allow(deprecated)]
fn transfer_authority_rejects_zero_contract_address() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, authority, _) = deploy_governor(&env);
    let zero_contract = Address::from_string(&soroban_sdk::String::from_str(
        &env,
        "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4",
    ));

    let result = client.try_transfer_authority(&authority, &zero_contract);
    assert_eq!(result, Err(Ok(Error::InvalidParam)));
}
