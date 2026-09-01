use soroban_sdk::{symbol_short, Address, Env};

use crate::Role;

pub fn initialized(
    env: &Env,
    authority: &Address,
    fee_recipient: &Address,
    factory_address: &Address,
) {
    env.events().publish(
        (symbol_short!("init"), authority.clone()),
        (fee_recipient.clone(), factory_address.clone()),
    );
}

/// Emitted when an `Admin` grants `role` to `account`.
///
/// # Indexer & Context Note
/// Unlike single-holder governance parameters (such as `fee_recipient` or `fee_bps`),
/// roles in `DripGovernor` are multi-holder role sets (multiple accounts can hold
/// `Admin`, `FeeManager`, or `RateManager` concurrently).
///
/// Role-change events emit self-contained membership deltas:
/// - Topic: `("grant", caller)` — identifies the authorizing Admin.
/// - Data: `(role, account)` — specifies the exact role and grantee account being added.
///
/// Off-chain indexers maintain active role membership by tracking `grant` (add)
/// and `revoke` (remove) deltas. Events are emitted only when an actual state
/// transition occurs (i.e. when `account` did not already hold `role`).
pub fn grant_role(env: &Env, caller: &Address, role: Role, account: &Address) {
    env.events().publish(
        (symbol_short!("grant"), caller.clone()),
        (role, account.clone()),
    );
}

/// Emitted when an `Admin` revokes `role` from `account`.
///
/// # Indexer & Context Note
/// Role-change events emit self-contained membership deltas for multi-holder RBAC:
/// - Topic: `("revoke", caller)` — identifies the authorizing Admin.
/// - Data: `(role, account)` — specifies the exact role and account being removed.
///
/// Off-chain indexers remove `account` from the active holder set for `role` upon
/// observing this event. Events are emitted only when an actual state transition
/// occurs (i.e. when `account` held `role` prior to revocation).
pub fn revoke_role(env: &Env, caller: &Address, role: Role, account: &Address) {
    env.events().publish(
        (symbol_short!("revoke"), caller.clone()),
        (role, account.clone()),
    );
}

pub fn transfer_authority(env: &Env, caller: &Address, new_authority: &Address) {
    env.events().publish(
        (symbol_short!("transfer"), caller.clone()),
        new_authority.clone(),
    );
}

/// Emitted when an `Admin` proposes a new authority address.
///
/// # Indexer & Context Note
/// This is the first step of a 2-step authority transfer (Ownable2Step pattern).
/// The transfer is not complete until `accept_authority` is called by the
/// proposed address.
pub fn propose_authority(env: &Env, caller: &Address, new_authority: &Address) {
    env.events().publish(
        (symbol_short!("propose"), caller.clone()),
        new_authority.clone(),
    );
}

/// Emitted when the pending authority accepts the transfer.
///
/// # Indexer & Context Note
/// This completes the 2-step authority transfer. The new authority now holds
/// the Admin role, and the old authority's Admin role is revoked.
pub fn accept_authority(env: &Env, caller: &Address, old_authority: &Address) {
    env.events().publish(
        (symbol_short!("accept"), caller.clone()),
        old_authority.clone(),
    );
}

pub fn set_fee_bps(env: &Env, caller: &Address, old_fee_bps: u32, new_fee_bps: u32) {
    env.events().publish(
        (symbol_short!("fee_bps"), caller.clone()),
        (old_fee_bps, new_fee_bps),
    );
}

pub fn set_fee_recipient(
    env: &Env,
    caller: &Address,
    old_recipient: &Address,
    new_recipient: &Address,
) {
    env.events().publish(
        (symbol_short!("fee_rec"), caller.clone()),
        (old_recipient.clone(), new_recipient.clone()),
    );
}

pub fn set_min_duration(env: &Env, caller: &Address, old_seconds: u64, new_seconds: u64) {
    env.events().publish(
        (symbol_short!("min_dur"), caller.clone()),
        (old_seconds, new_seconds),
    );
}

pub fn set_max_rate(env: &Env, caller: &Address, old_max_rate: i128, new_max_rate: i128) {
    env.events().publish(
        (symbol_short!("max_rate"), caller.clone()),
        (old_max_rate, new_max_rate),
    );
}

pub fn set_max_duration(env: &Env, caller: &Address, old_seconds: u64, new_seconds: u64) {
    env.events().publish(
        (symbol_short!("max_dur"), caller.clone()),
        (old_seconds, new_seconds),
    );
}

pub fn set_force_cancel_pause_threshold(env: &Env, caller: &Address, seconds: u64) {
    env.events()
        .publish((symbol_short!("fc_pause"), caller.clone()), seconds);
}

pub fn paused(env: &Env, caller: &Address, paused_at: u64) {
    env.events()
        .publish((symbol_short!("paused"), caller.clone()), paused_at);
}

pub fn unpaused(env: &Env, caller: &Address, resumed_at: u64) {
    env.events()
        .publish((symbol_short!("unpaused"), caller.clone()), resumed_at);
}

/// Emitted after a successful `pause_factory` cross-contract call, so
/// off-chain infra can confirm the *factory's* pause committed, distinct
/// from `paused` (which marks the governor's own pause).
pub fn factory_paused(env: &Env, caller: &Address, paused_at: u64) {
    env.events()
        .publish((symbol_short!("fpaused"), caller.clone()), paused_at);
}

/// Emitted after a successful `unpause_factory` cross-contract call.
pub fn factory_unpaused(env: &Env, caller: &Address, resumed_at: u64) {
    env.events()
        .publish((symbol_short!("funpaused"), caller.clone()), resumed_at);
}

pub fn upgraded(env: &Env, caller: &Address, upgraded_at: u64) {
    env.events()
        .publish((symbol_short!("upgraded"), caller.clone()), upgraded_at);
}
