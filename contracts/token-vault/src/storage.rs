use soroban_sdk::{contracttype, Address, Env};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Owner,
    Token,
    MaxLimit,
    Balance,
    /// Optional operator address delegated by the owner.
    ///
    /// When set, the operator can perform owner-level actions (`withdraw`,
    /// `set_limit`) on behalf of the owner — a hot-wallet / ops-key
    /// pattern matching `DripStream`'s `set_operator` design.
    /// Absent key means no operator has been delegated.
    Operator,
    /// Maximum amount a delegated operator may withdraw in a single call.
    ///
    /// When set, `withdraw` enforces this cap for operator-authenticated
    /// withdrawals while the owner remains unbounded. If this key is absent,
    /// the operator is effectively unable to withdraw until the owner sets a
    /// positive cap.
    OperatorWithdrawLimit,
    /// Emergency-pause flag. When `true`, all state-mutating entry points
    /// (deposit, withdraw, set_limit) revert before touching state.
    Paused,
    /// Pending owner address for the 2-step owner transfer.
    /// Set by `propose_owner`, consumed by `accept_owner`.
    PendingOwner,
    /// The current owner who proposed the transfer.
    /// Used to hand ownership over when the pending owner accepts.
    PendingOwnerProposer,
}

pub fn set_owner(env: &Env, a: &Address) {
    env.storage().instance().set(&DataKey::Owner, a);
}

pub fn get_owner(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::Owner)
}

pub fn set_token(env: &Env, t: &Address) {
    env.storage().instance().set(&DataKey::Token, t);
}

pub fn get_token(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::Token)
}

pub fn set_max_limit(env: &Env, v: &i128) {
    env.storage().instance().set(&DataKey::MaxLimit, v);
}

pub fn get_max_limit(env: &Env) -> Option<i128> {
    env.storage().instance().get(&DataKey::MaxLimit)
}

#[allow(dead_code)]
pub fn set_balance(env: &Env, v: &i128) {
    env.storage().instance().set(&DataKey::Balance, v);
}

#[allow(dead_code)]
pub fn get_balance(env: &Env) -> Option<i128> {
    env.storage().instance().get(&DataKey::Balance)
}

#[allow(dead_code)]
pub fn remove_operator_withdraw_limit(env: &Env) {
    env.storage()
        .instance()
        .remove(&DataKey::OperatorWithdrawLimit);
}

pub fn set_operator(env: &Env, op: &Address) {
    env.storage().instance().set(&DataKey::Operator, op);
}

pub fn get_operator(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::Operator)
}

pub fn remove_operator(env: &Env) {
    env.storage().instance().remove(&DataKey::Operator);
}

pub fn set_operator_withdraw_limit(env: &Env, v: &i128) {
    env.storage()
        .instance()
        .set(&DataKey::OperatorWithdrawLimit, v);
}

pub fn get_operator_withdraw_limit(env: &Env) -> Option<i128> {
    env.storage()
        .instance()
        .get(&DataKey::OperatorWithdrawLimit)
}

pub fn set_paused(env: &Env, paused: bool) {
    env.storage().instance().set(&DataKey::Paused, &paused);
}

pub fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false)
}

pub fn set_pending_owner(env: &Env, owner: &Address) {
    env.storage().instance().set(&DataKey::PendingOwner, owner);
}

pub fn get_pending_owner(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::PendingOwner)
}

pub fn remove_pending_owner(env: &Env) {
    env.storage().instance().remove(&DataKey::PendingOwner);
}

pub fn set_pending_owner_proposer(env: &Env, proposer: &Address) {
    env.storage()
        .instance()
        .set(&DataKey::PendingOwnerProposer, proposer);
}

pub fn get_pending_owner_proposer(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::PendingOwnerProposer)
}

pub fn remove_pending_owner_proposer(env: &Env) {
    env.storage()
        .instance()
        .remove(&DataKey::PendingOwnerProposer);
}
