//! Event emission for `TokenVault` state-mutating operations.
//!
//! Mirrors the dedicated `events` modules in `DripStream`, `DripFactory`,
//! `DripGovernor`, and `TwapOracle`: every state-mutating entry point
//! publishes an event so off-chain indexers and relayers can observe vault
//! activity without diffing storage or watching the underlying token's own
//! transfer events.
//!
//! Publication and the corresponding storage write are part of the same
//! Soroban transaction, so either both commit or both roll back — an event
//! is never emitted for a transition that did not actually persist.

use soroban_sdk::{symbol_short, Address, Env};

/// Emitted by `initialize` when the vault is first set up.
///
/// Topics: `("init", owner)` — the address that owns the vault.
/// Data:   `(token, max_limit)` — the escrowed asset and initial cap.
pub fn initialized(env: &Env, owner: &Address, token: &Address, max_limit: i128) {
    env.events().publish(
        (symbol_short!("init"), owner.clone()),
        (token.clone(), max_limit),
    );
}

/// Emitted by `deposit` after tokens are moved into the vault.
///
/// Topics: `("deposited", from)` — the address that funded the deposit.
/// Data:   `(amount, new_balance)` — the deposited amount and the vault's
/// resulting total balance.
pub fn deposited(env: &Env, from: &Address, amount: i128, new_balance: i128) {
    env.events().publish(
        (symbol_short!("deposited"), from.clone()),
        (amount, new_balance),
    );
}

/// Emitted by `withdraw` after tokens are paid out of the vault.
///
/// Topics: `("withdrawn", caller)` — the owner or delegated operator that
/// authorized the withdrawal.
/// Data:   `(to, amount, new_balance)` — the recipient, the withdrawn
/// amount, and the vault's resulting total balance.
pub fn withdrawn(env: &Env, caller: &Address, to: &Address, amount: i128, new_balance: i128) {
    env.events().publish(
        (symbol_short!("withdrawn"), caller.clone()),
        (to.clone(), amount, new_balance),
    );
}

/// Emitted by `set_limit` when the vault's maximum balance cap changes.
///
/// Topics: `("limit_set", caller)` — the owner or delegated operator.
/// Data:   `(old_limit, new_limit)` — the previous and new caps, so
/// indexers can reconstruct the full limit history without a state read.
pub fn limit_set(env: &Env, caller: &Address, old_limit: i128, new_limit: i128) {
    env.events().publish(
        (symbol_short!("limit_set"), caller.clone()),
        (old_limit, new_limit),
    );
}

/// Emitted by `set_operator_withdraw_limit` when the owner configures how much
/// a delegated operator may withdraw in a single call.
pub fn operator_withdraw_limit_set(env: &Env, caller: &Address, old_limit: i128, new_limit: i128) {
    env.events().publish(
        (symbol_short!("op_limit"), caller.clone()),
        (old_limit, new_limit),
    );
}

/// Emitted by `set_operator` when the owner delegates a new operator.
///
/// Topics: `("set_op", caller)` — the owner.
/// Data:   `operator` — the newly delegated operator address.
pub fn operator_set(env: &Env, caller: &Address, operator: &Address) {
    env.events()
        .publish((symbol_short!("set_op"), caller.clone()), operator.clone());
}

/// Emitted by `revoke_operator` when the owner removes the operator.
///
/// Topics: `("rm_op", caller)` — the owner.
/// Data:   none.
pub fn operator_revoked(env: &Env, caller: &Address) {
    env.events()
        .publish((symbol_short!("rm_op"), caller.clone()), ());
}

/// Emitted when the vault transitions from unpaused to paused.
///
/// Topics: `("paused", caller)` — the owner that authorized the halt.
/// Data:   `paused_at` — the ledger timestamp at which the halt took
/// effect, so off-chain infra can positively confirm the transition
/// committed rather than inferring it from a bare `Ok`.
pub fn paused(env: &Env, caller: &Address, paused_at: u64) {
    env.events()
        .publish((symbol_short!("paused"), caller.clone()), paused_at);
}

/// Emitted when the vault transitions from paused back to unpaused.
///
/// Topics: `("unpaused", caller)` — the owner that lifted the halt.
/// Data:   `resumed_at` — the ledger timestamp at which operations resumed.
pub fn unpaused(env: &Env, caller: &Address, resumed_at: u64) {
    env.events()
        .publish((symbol_short!("unpaused"), caller.clone()), resumed_at);
}

/// Emitted by `propose_owner` (step 1 of the 2-step owner transfer).
///
/// Topics: `("propose", caller)` — the current owner.
/// Data:   `new_owner` — the proposed pending owner address.
///
/// The transfer is not complete until `accept_owner` is called by the
/// proposed address. Mirrors `DripGovernor`'s `propose_authority`.
pub fn owner_proposed(env: &Env, caller: &Address, new_owner: &Address) {
    env.events().publish(
        (symbol_short!("propose"), caller.clone()),
        new_owner.clone(),
    );
}

/// Emitted by `accept_owner` (step 2 of the 2-step owner transfer).
///
/// Topics: `("accept", caller)` — the new owner that accepted the transfer.
/// Data:   `old_owner` — the previous owner, who loses ownership.
pub fn owner_accepted(env: &Env, caller: &Address, old_owner: &Address) {
    env.events()
        .publish((symbol_short!("accept"), caller.clone()), old_owner.clone());
}
