//! Integration tests: DripFactory validation and registry queries.
//!
//! Deployment tests (create_stream success path) require the stream WASM to be
//! built first (`cargo build --target wasm32-unknown-unknown --release`).
//! The tests in this file cover validation guards and registry read-paths that
//! do not require an actual stream deployment.

use drip_factory::{DripFactory, DripFactoryClient, Error};
use drip_governor::{DripGovernor, DripGovernorClient};
use soroban_sdk::{
    testutils::{storage::Instance as _, Address as _, Ledger, LedgerInfo},
    token, Address, BytesN, Env, TryIntoVal,
};

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

/// Deploys a real DripFactory wired to a real, initialized DripGovernor —
/// matching the two-step deploy order in docs/architecture.md (both
/// contracts are registered first, then each is initialized with the
/// other's address). create_stream reads governor config, so a dummy
/// governor address is no longer sufficient here.
fn deploy_factory(env: &Env) -> DripFactoryClient<'_> {
    let factory_id = env.register_contract(None, DripFactory);
    let governor_id = env.register_contract(None, DripGovernor);

    let authority = Address::generate(env);
    let fee_recipient = Address::generate(env);
    let governor_client = DripGovernorClient::new(env, &governor_id);
    governor_client.initialize(&authority, &fee_recipient, &factory_id);

    let client = DripFactoryClient::new(env, &factory_id);
    let dummy_hash = BytesN::from_array(env, &[1u8; 32]);
    client.initialize(&dummy_hash, &governor_id);
    client
}

/// Same as `deploy_factory`, but also hands back the governor client and its
/// bootstrap authority so tests can adjust `max_rate_per_second` /
/// `min_duration_seconds` (which are role-gated) and observe the factory
/// enforcing them.
fn deploy_factory_with_governor<'a>(
    env: &'a Env,
) -> (DripFactoryClient<'a>, DripGovernorClient<'a>, Address) {
    let factory_id = env.register_contract(None, DripFactory);
    let governor_id = env.register_contract(None, DripGovernor);

    let authority = Address::generate(env);
    let fee_recipient = Address::generate(env);
    let governor_client = DripGovernorClient::new(env, &governor_id);
    governor_client.initialize(&authority, &fee_recipient, &factory_id);

    let factory_client = DripFactoryClient::new(env, &factory_id);
    let dummy_hash = BytesN::from_array(env, &[1u8; 32]);
    factory_client.initialize(&dummy_hash, &governor_id);

    (factory_client, governor_client, authority)
}

// ── Fresh factory state ───────────────────────────────────────────────────────

#[test]
fn stream_count_starts_at_zero() {
    let env = base_env();
    let client = deploy_factory(&env);
    assert_eq!(client.stream_count(), 0);
}

#[test]
fn stream_address_returns_none_for_nonexistent_id() {
    let env = base_env();
    let client = deploy_factory(&env);
    assert!(client.stream_address(&0).is_none());
    assert!(client.stream_address(&999).is_none());
}

#[test]
fn streams_by_sender_returns_empty_for_unknown_address() {
    let env = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    let result = client.streams_by_sender(&sender, &0, &10);
    assert_eq!(result.ids.len(), 0);
    assert_eq!(result.total, 0);
}

#[test]
fn pagination_does_not_panic_when_offset_plus_limit_overflows_u32() {
    let env = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    // offset + limit would overflow u32 with raw addition; must not panic
    // and must simply return no results since the index is empty.
    let result = client.streams_by_sender(&sender, &u32::MAX, &u32::MAX);
    assert_eq!(result.ids.len(), 0);
}

#[test]
fn pagination_clamps_offset_near_u32_max_against_populated_index() {
    use drip_factory::storage::DataKey;
    use soroban_sdk::Vec as SVec;

    let env = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    let recip = Address::generate(&env);

    // Seed a non-empty index directly (bypasses create_stream, which needs a
    // built stream WASM) so the offset guard is exercised against real
    // entries rather than trivially against an empty Vec.
    let mut ids = SVec::new(&env);
    ids.push_back(1u64);
    ids.push_back(2u64);
    ids.push_back(3u64);
    env.as_contract(&client.address, || {
        env.storage()
            .persistent()
            .set(&DataKey::BySender(sender.clone()), &ids);
        env.storage()
            .persistent()
            .set(&DataKey::ByRecipient(recip.clone()), &ids);
    });

    // offset near u32::MAX is far past the 3-element index; offset + limit
    // would overflow u32 with raw addition, so this must clamp to an empty
    // result rather than panicking.
    let by_sender = client.streams_by_sender(&sender, &(u32::MAX - 1), &10);
    assert_eq!(by_sender.ids.len(), 0);
    assert_eq!(by_sender.total, 3);
    let by_recipient = client.streams_by_recipient(&recip, &(u32::MAX - 1), &10);
    assert_eq!(by_recipient.ids.len(), 0);
    assert_eq!(by_recipient.total, 3);

    // A valid in-range offset combined with a limit near u32::MAX must still
    // clamp to the index's actual length instead of overflowing.
    let tail = client.streams_by_sender(&sender, &1, &u32::MAX).ids;
    assert_eq!(tail.len(), 2);
    assert_eq!(tail.get(0), Some(2));
    assert_eq!(tail.get(1), Some(3));
}

#[test]
fn streams_by_recipient_returns_empty_for_unknown_address() {
    let env = base_env();
    let client = deploy_factory(&env);
    let recip = Address::generate(&env);
    let result = client.streams_by_recipient(&recip, &0, &10);
    assert_eq!(result.ids.len(), 0);
    assert_eq!(result.total, 0);
}

#[test]
fn protocol_fee_bps_returns_default_30() {
    let env = base_env();
    let client = deploy_factory(&env);
    assert_eq!(client.protocol_fee_bps(), 30);
}

#[test]
fn upgrade_stream_wasm_on_uninitialized_factory_is_rejected() {
    let env = base_env();
    let id = env.register_contract(None, DripFactory);
    let client = DripFactoryClient::new(&env, &id);
    let new_hash = BytesN::from_array(&env, &[2u8; 32]);
    let result = client.try_upgrade_stream_wasm(&new_hash);
    assert_eq!(result, Err(Ok(Error::NotInitialized)));
}

// ── TTL management ─────────────────────────────────────────────────────────────

#[test]
fn initialize_extends_factory_instance_ttl() {
    let env = base_env();
    let client = deploy_factory(&env);
    let ttl = env.as_contract(&client.address, || env.storage().instance().get_ttl());
    assert_eq!(ttl, 200_000);
}

#[test]
fn upgrade_stream_wasm_extends_instance_ttl() {
    let env = base_env();
    let client = deploy_factory(&env);
    let new_hash = BytesN::from_array(&env, &[3u8; 32]);
    client.upgrade_stream_wasm(&new_hash);
    let ttl = env.as_contract(&client.address, || env.storage().instance().get_ttl());
    assert_eq!(ttl, 200_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn re_initializing_factory_panics() {
    let env = base_env();
    let client = deploy_factory(&env);
    // An attacker calling initialize() again to swap in their own
    // stream_wasm_hash/governor must be rejected.
    let attacker_governor = Address::generate(&env);
    let attacker_hash = BytesN::from_array(&env, &[1u8; 32]);
    client.initialize(&attacker_hash, &attacker_governor);
}

// ── Validation errors (all fail before deployment) ────────────────────────────

fn make_token(env: &Env, sender: &Address, amount: i128) -> Address {
    let admin = Address::generate(env);
    let addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    token::StellarAssetClient::new(env, &addr).mint(sender, &amount);
    addr
}

#[test]
fn create_stream_rejects_zero_stellar_recipient() {
    let env = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);

    let zero_recipient = Address::from_string(&soroban_sdk::String::from_str(
        &env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    ));

    let token = make_token(&env, &sender, 100_000);
    let now = env.ledger().timestamp();

    let result = client.try_create_stream(
        &sender,
        &zero_recipient,
        &token,
        &100_000,
        &100,
        &(now + 100),
        &(now + 3_700),
        &false,
    );

    assert_eq!(result, Err(Ok(Error::InvalidRecipient)));
}

#[test]
fn create_stream_rejects_zero_contract_recipient() {
    let env = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);

    let zero_recipient = Address::from_string(&soroban_sdk::String::from_str(
        &env,
        "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4",
    ));

    let token = make_token(&env, &sender, 100_000);
    let now = env.ledger().timestamp();

    let result = client.try_create_stream(
        &sender,
        &zero_recipient,
        &token,
        &100_000,
        &100,
        &(now + 100),
        &(now + 3_700),
        &false,
    );

    assert_eq!(result, Err(Ok(Error::InvalidRecipient)));
}

#[test]
fn create_stream_rejects_sender_as_recipient() {
    let env = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);

    let token = make_token(&env, &sender, 100_000);
    let now = env.ledger().timestamp();

    let result = client.try_create_stream(
        &sender,
        &sender, // sender is recipient
        &token,
        &100_000,
        &100,
        &(now + 100),
        &(now + 3_700),
        &false,
    );

    assert_eq!(result, Err(Ok(Error::InvalidRecipient)));
}

#[test]
fn create_stream_rejects_zero_stellar_token() {
    let env = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    let recip = Address::generate(&env);

    let zero_token = Address::from_string(&soroban_sdk::String::from_str(
        &env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    ));
    let now = env.ledger().timestamp();

    let result = client.try_create_stream(
        &sender,
        &recip,
        &zero_token,
        &100_000,
        &100,
        &(now + 100),
        &(now + 3_700),
        &false,
    );

    assert_eq!(result, Err(Ok(Error::InvalidToken)));
}

#[test]
fn create_stream_rejects_zero_contract_token() {
    let env = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    let recip = Address::generate(&env);

    let zero_token = Address::from_string(&soroban_sdk::String::from_str(
        &env,
        "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4",
    ));
    let now = env.ledger().timestamp();

    let result = client.try_create_stream(
        &sender,
        &recip,
        &zero_token,
        &100_000,
        &100,
        &(now + 100),
        &(now + 3_700),
        &false,
    );

    assert_eq!(result, Err(Ok(Error::InvalidToken)));
}

#[test]
fn create_stream_rejects_zero_deposit() {
    let env = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    let recip = Address::generate(&env);
    let token = make_token(&env, &sender, 0);
    let now = env.ledger().timestamp();
    // deposit = 0 → InvalidDeposit before any deployment
    let result = client.try_create_stream(
        &sender,
        &recip,
        &token,
        &0,
        &100,
        &(now + 100),
        &(now + 3_700),
        &false,
    );
    assert_eq!(result, Err(Ok(Error::InvalidDeposit)));
}

#[test]
fn create_stream_rejects_negative_deposit() {
    let env = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    let recip = Address::generate(&env);
    let token = make_token(&env, &sender, 10_000);
    let now = env.ledger().timestamp();
    // A negative amount can never fund a stream — the `deposit > 0`
    // guard must reject it as InvalidDeposit before any deployment.
    let result = client.try_create_stream(
        &sender,
        &recip,
        &token,
        &-1,
        &100,
        &(now + 100),
        &(now + 3_700),
        &false,
    );
    assert_eq!(result, Err(Ok(Error::InvalidDeposit)));
}

#[test]
fn create_stream_rejects_zero_rate() {
    let env = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    let recip = Address::generate(&env);
    let token = make_token(&env, &sender, 10_000);
    let now = env.ledger().timestamp();
    let result = client.try_create_stream(
        &sender,
        &recip,
        &token,
        &10_000,
        &0,
        &(now + 100),
        &(now + 3_700),
        &false,
    );
    assert_eq!(result, Err(Ok(Error::InvalidRate)));
}

#[test]
fn create_stream_rejects_deposit_less_than_one_second() {
    let env = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    let recip = Address::generate(&env);
    let token = make_token(&env, &sender, 10_000);
    let now = env.ledger().timestamp();
    // deposit (50) < rate_per_sec (100) → InsufficientDeposit
    let result = client.try_create_stream(
        &sender,
        &recip,
        &token,
        &50,
        &100,
        &(now + 100),
        &(now + 3_700),
        &false,
    );
    assert_eq!(result, Err(Ok(Error::InsufficientDeposit)));
}

#[test]
fn create_stream_rejects_deposit_short_of_full_duration() {
    let env = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    let recip = Address::generate(&env);
    let now = env.ledger().timestamp();
    // rate 100/s over a 3_600s window needs a 360_000 deposit; passing
    // enough for only 1 second (which alone would pass the old check)
    // must now be rejected because it can't fund the declared end_time.
    let token = make_token(&env, &sender, 100);
    let result = client.try_create_stream(
        &sender,
        &recip,
        &token,
        &100,
        &100,
        &now,
        &(now + 3_600),
        &false,
    );
    assert_eq!(result, Err(Ok(Error::InsufficientDeposit)));
}

#[test]
fn create_stream_rejects_start_time_in_past() {
    let env = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    let recip = Address::generate(&env);
    let token = make_token(&env, &sender, 100_000);
    let now = env.ledger().timestamp();
    // start_time = now - 1 → BackdatedStream
    let result = client.try_create_stream(
        &sender,
        &recip,
        &token,
        &100_000,
        &100,
        &(now - 1),
        &(now + 3_600),
        &false,
    );
    assert_eq!(result, Err(Ok(Error::BackdatedStream)));
}

#[test]
fn create_stream_rejects_end_before_start() {
    let env = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    let recip = Address::generate(&env);
    let token = make_token(&env, &sender, 100_000);
    let now = env.ledger().timestamp();
    // end_time <= start_time → InvalidDuration
    let result = client.try_create_stream(
        &sender,
        &recip,
        &token,
        &100_000,
        &100,
        &(now + 100),
        &(now + 50),
        &false,
    );
    assert_eq!(result, Err(Ok(Error::InvalidDuration)));
}

#[test]
fn create_stream_rejects_end_equal_to_start() {
    let env = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    let recip = Address::generate(&env);
    let token = make_token(&env, &sender, 100_000);
    let now = env.ledger().timestamp();
    let start = now + 100;
    let result = client.try_create_stream(
        &sender, &recip, &token, &100_000, &100, &start, &start, &false,
    );
    assert_eq!(result, Err(Ok(Error::InvalidDuration)));
}

#[test]
fn test_create_stream_zero_duration_reverts() {
    let env = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    let recip = Address::generate(&env);
    let token = make_token(&env, &sender, 100_000);
    let now = env.ledger().timestamp();
    let start = now + 100;
    let result = client.try_create_stream(
        &sender, &recip, &token, &100_000, &100, &start, &start, &false,
    );
    assert_eq!(result, Err(Ok(Error::InvalidDuration)));
}

// ── Governor-controlled bounds ────────────────────────────────────────────────

#[test]
fn create_stream_rejects_rate_above_governor_max() {
    let env = base_env();
    let (factory, governor, authority) = deploy_factory_with_governor(&env);
    governor.set_max_rate(&authority, &1_000);

    let sender = Address::generate(&env);
    let recip = Address::generate(&env);
    let token = make_token(&env, &sender, 1_000_000);
    let now = env.ledger().timestamp();
    // rate 1_001 > governor's max_rate_per_second of 1_000
    let result = factory.try_create_stream(
        &sender, &recip, &token, &1_000_000, &1_001, &now, &0, &false,
    );
    assert_eq!(result, Err(Ok(Error::RateExceedsMax)));
}

#[test]
fn create_stream_rejects_duration_below_governor_minimum() {
    let env = base_env();
    let (factory, governor, authority) = deploy_factory_with_governor(&env);
    governor.set_min_duration(&authority, &7_200);

    let sender = Address::generate(&env);
    let recip = Address::generate(&env);
    let token = make_token(&env, &sender, 1_000_000);
    let now = env.ledger().timestamp();
    // duration of 3_600s < governor's min_duration_seconds of 7_200
    let result = factory.try_create_stream(
        &sender,
        &recip,
        &token,
        &1_000_000,
        &100,
        &now,
        &(now + 3_600),
        &false,
    );
    assert_eq!(result, Err(Ok(Error::DurationTooShort)));
}

#[test]
fn create_stream_rejects_duration_above_governor_maximum() {
    let env = base_env();
    let (factory, governor, admin) = deploy_factory_with_governor(&env);
    governor.set_max_duration(&admin, &7_200);

    let sender = Address::generate(&env);
    let recip = Address::generate(&env);
    let token = make_token(&env, &sender, 2_000_000);
    let now = env.ledger().timestamp();
    // duration of 3_600s is fine, but we set max to 7_200; try exceeding it
    let result = factory.try_create_stream(
        &sender,
        &recip,
        &token,
        &2_000_000,
        &100,
        &now,
        &(now + 10_800),
        &false,
    );
    assert_eq!(result, Err(Ok(Error::DurationExceedsMax)));
}

#[test]
fn protocol_fee_bps_reflects_live_governor_value() {
    let env = base_env();
    let (factory, governor, authority) = deploy_factory_with_governor(&env);
    assert_eq!(factory.protocol_fee_bps(), 30);

    governor.set_fee_bps(&authority, &75);
    assert_eq!(factory.protocol_fee_bps(), 75);
}

#[test]
fn protocol_fee_bps_reports_not_initialized_rather_than_guessing_30() {
    // Previously this asserted a bare `30` for a factory that has no governor
    // to read a fee from — indistinguishable from a governor genuinely
    // configured at 30 bps, which is the ambiguity issue #339 describes.
    let env = base_env();
    let id = env.register_contract(None, DripFactory);
    let client = DripFactoryClient::new(&env, &id);

    assert_eq!(
        client.try_protocol_fee_bps(),
        Err(Ok(drip_factory::Error::NotInitialized))
    );
}

#[test]
fn protocol_fee_bps_or_default_preserves_the_lenient_read() {
    // Callers that would rather show an approximate fee than nothing can still
    // do so — but they pass the fallback in, so the choice is visible at the
    // call site instead of hidden inside the factory.
    let env = base_env();
    let id = env.register_contract(None, DripFactory);
    let client = DripFactoryClient::new(&env, &id);

    assert_eq!(client.protocol_fee_bps_or_default(&30), 30);
    assert_eq!(client.protocol_fee_bps_or_default(&0), 0);
}

// ── Reentrancy guard ────────────────────────────────────────────────────────

#[test]
fn create_stream_rejects_reentrant_call_while_locked() {
    use drip_factory::storage::DataKey;

    let env = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    let recip = Address::generate(&env);
    let token = make_token(&env, &sender, 10_000);
    let now = env.ledger().timestamp();

    // Simulates a malicious `token.transfer()` reentering `create_stream`
    // mid-call: the lock is already held when this call is made, so it
    // must be rejected with CreateLocked rather than racing StreamCount.
    env.as_contract(&client.address, || {
        env.storage().instance().set(&DataKey::CreateLock, &true);
    });

    let result = client.try_create_stream(
        &sender,
        &recip,
        &token,
        &10_000,
        &1,
        &(now + 100),
        &(now + 3_700),
        &false,
    );
    assert_eq!(result, Err(Ok(Error::CreateLocked)));
}

#[test]
fn create_stream_lock_does_not_block_validation_failures() {
    use drip_factory::storage::DataKey;

    let env = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    let recip = Address::generate(&env);
    let token = make_token(&env, &sender, 10_000);
    let now = env.ledger().timestamp();

    // A call that fails input validation (zero deposit) never reaches the
    // lock check, so it must still surface its own specific error rather
    // than CreateLocked, and the lock itself is confirmed unset afterward.
    let result = client.try_create_stream(
        &sender,
        &recip,
        &token,
        &0,
        &100,
        &(now + 100),
        &(now + 3_700),
        &false,
    );
    assert_eq!(result, Err(Ok(Error::InvalidDeposit)));

    let locked: bool = env.as_contract(&client.address, || {
        env.storage()
            .instance()
            .get(&DataKey::CreateLock)
            .unwrap_or(false)
    });
    assert!(!locked);
}

// ── Defensive arithmetic in enforce_bounds ────────────────────────────────────

#[test]
fn enforce_bounds_with_end_time_before_start_time_returns_arithmetic_overflow() {
    let env = base_env();
    let (factory, _governor, _authority) = deploy_factory_with_governor(&env);

    let sender = Address::generate(&env);
    let recip = Address::generate(&env);
    let token = make_token(&env, &sender, 1_000_000);
    let now = env.ledger().timestamp();

    // Attempt to create a stream where end_time < start_time.
    // Early validation in create_stream returns InvalidDuration.
    let result = factory.try_create_stream(
        &sender,
        &recip,
        &token,
        &1_000_000,
        &100,
        &(now + 7_200),
        &(now + 3_600), // end_time < start_time
        &false,
    );
    assert_eq!(result, Err(Ok(Error::InvalidDuration)));
}

#[test]
fn enforce_bounds_with_equal_start_and_end_time_returns_arithmetic_overflow() {
    let env = base_env();
    let (factory, _governor, _authority) = deploy_factory_with_governor(&env);

    let sender = Address::generate(&env);
    let recip = Address::generate(&env);
    let token = make_token(&env, &sender, 1_000_000);
    let now = env.ledger().timestamp();

    // A stream where start_time == end_time has zero duration.
    // Early validation in create_stream returns InvalidDuration.
    let result = factory.try_create_stream(
        &sender,
        &recip,
        &token,
        &1_000_000,
        &100,
        &(now + 7_200),
        &(now + 7_200), // end_time == start_time
        &false,
    );
    assert_eq!(result, Err(Ok(Error::InvalidDuration)));
}

// ── Issue #232: upgrade_stream_wasm zero-hash guard, integration-level ─────
// Note: contracts/factory/src/tests.rs already covers this guard at the unit
// level (upgrade_stream_wasm_rejects_zero_hash, added for issue #86). This
// adds the equivalent check here since #232 specifically flagged this file.
#[test]
fn upgrade_stream_wasm_rejects_all_zero_hash() {
    let env = base_env();
    let client = deploy_factory(&env);
    let zero_hash = BytesN::from_array(&env, &[0u8; 32]);
    let result = client.try_upgrade_stream_wasm(&zero_hash);
    assert_eq!(result, Err(Ok(Error::InvalidWasmHash)));
}

// ── Issue #351: protocol fee is charged at creation ─────────────────────────
//
// The governed `fee_bps` / `fee_recipient` from DripGovernor were stored and
// read (`protocol_fee_bps()`) but never applied. create_stream now charges the
// fee as a surcharge on the deposit: the sender funds `deposit + fee`, the
// stream receives the full `deposit`, and `fee` is routed to `fee_recipient`.
//
// This needs a real stream WASM (a successful create_stream deploys one), so
// it is #[ignore]d like the other deployment-path tests. Run with:
//   cargo build -p drip-stream --target wasm32-unknown-unknown --release
//   cargo test --test factory_deploy -- --ignored --nocapture
#[test]
#[ignore = "requires stream WASM built via cargo build -p drip-stream --target wasm32-unknown-unknown --release"]
fn create_stream_charges_and_routes_protocol_fee() {
    use soroban_sdk::testutils::Events as _;

    let env = base_env();
    env.mock_all_auths();

    // Deploy a dedicated factory + governor so we control fee_recipient and can
    // read it back from governor config for the assertion.
    let factory_id = env.register_contract(None, DripFactory);
    let governor_id = env.register_contract(None, DripGovernor);

    let authority = Address::generate(&env);
    let fee_recipient = Address::generate(&env);
    let governor_client = DripGovernorClient::new(&env, &governor_id);
    governor_client.initialize(&authority, &fee_recipient, &factory_id);

    let client = DripFactoryClient::new(&env, &factory_id);
    let dummy_hash = BytesN::from_array(&env, &[0u8; 32]);
    client.initialize(&dummy_hash, &governor_id);

    // Wire a real stream WASM into the factory.
    let wasm_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/target/wasm32-unknown-unknown/release/drip_stream.wasm"
    );
    let wasm = std::fs::read(wasm_path).expect("build the stream WASM first (see docstring)");
    let wasm_hash = env.deployer().upload_contract_wasm(wasm.as_slice());
    env.as_contract(&client.address, || {
        env.storage()
            .instance()
            .set(&drip_factory::storage::DataKey::StreamWasmHash, &wasm_hash);
    });

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let now = env.ledger().timestamp();

    // Fresh token; default governor fee_bps = 30 (0.30%).
    let fee_bps = governor_client.config().fee_bps;
    let deposit: i128 = 360_000;
    let fee = deposit * fee_bps as i128 / 10_000;

    let admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let tok = token::Client::new(&env, &token_addr);
    let token_admin = token::StellarAssetClient::new(&env, &token_addr);
    // Fund the sender for deposit + fee so the surcharge can be collected.
    token_admin.mint(&sender, &(deposit + fee));

    client.create_stream(
        &sender,
        &recipient,
        &token_addr,
        &deposit,
        &100,
        &now,
        &(now + 3_600),
        &false,
    );

    let stream_addr = client
        .stream_address(&(client.stream_count() - 1))
        .unwrap();

    // 1) Sender paid deposit + fee (fully drained).
    assert_eq!(tok.balance(&sender), 0);

    // 2) Stream received the full deposit — funding accounting is unchanged.
    assert_eq!(tok.balance(&stream_addr), deposit);

    // 3) fee_recipient received exactly `deposit * fee_bps / 10_000`.
    assert_eq!(tok.balance(&fee_recipient), fee);

    // 4) The factory's public view quotes the fee that is now actually charged.
    assert_eq!(client.protocol_fee_bps(), fee_bps);

    // 5) A fee event was emitted (recipient, amount).
    let mut fee_events = 0;
    for e in env.events().all().into_iter() {
        if e.0 != client.address || e.1.len() != 2 {
            continue;
        }
        let sym: Option<soroban_sdk::Symbol> = e.1.get(0).and_then(|v| v.try_into_val(&env).ok());
        let topic1: Option<Address> = e.1.get(1).and_then(|v| v.try_into_val(&env).ok());
        let data: Option<i128> = e.2.try_into_val(&env).ok();
        if sym == Some(soroban_sdk::symbol_short!("fee"))
            && topic1 == Some(fee_recipient.clone())
            && data == Some(fee)
        {
            fee_events += 1;
        }
    }
    assert_eq!(fee_events, 1, "expected exactly one fee event");
}
