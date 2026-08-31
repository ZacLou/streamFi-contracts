//! Shared role-based access control (RBAC) for the Drip protocol.
//!
//! # Overview
//!
//! Both `DripGovernor` and `DripOracle` implement identical RBAC mechanics:
//!
//! - A set of roles, each of which may be held by any number of accounts.
//! - An `Admin` super-user that can grant/revoke any role (including `Admin`).
//! - A `LastAdmin` guard: the final `Admin` can never be revoked, so governance
//!   can never be permanently frozen.
//! - A `RoleMembers` index maintained on every `grant`/`revoke` so membership
//!   can be enumerated on-chain without replaying events.
//! - A `require_role_or_admin` gate: the caller must hold the specific role *or*
//!   be an `Admin`.
//!
//! # Generic design
//!
//! All functions are generic over the storage key types (`RK`, `AK`, `MK`) so
//! each contract can supply its own `DataKey` variants without pulling in this
//! crate's key enum.  The only constraint is that the keys implement
//! `soroban_sdk`'s `IntoVal<Env, Val>` + `TryFromVal<Env, Val>` (which every
//! `#[contracttype]`-derived type satisfies automatically).
//!
//! # TTL bumping
//!
//! `require_role_or_admin` accepts an optional `on_success: Option<fn(&Env)>`
//! callback. Pass `Some(ttl::bump)` in `DripGovernor` (which bumps instance TTL
//! on every successful role-gated write) or `None` in `DripOracle` (which bumps
//! TTL at the call-site entry point instead, before delegation).
//!
//! # Oracle-specific side-effects
//!
//! When `DripOracle` revokes a `PriceFeeder`, it must also purge the feeder's
//! `Submitters` entry and `Submission` record. That hook is deliberately kept
//! in `oracle/src/lib.rs` — the shared `revoke` function returns `true` when a
//! role was actually removed, and the oracle wrapper calls `remove_submitter`
//! only in that case.
//!
//! # Bug fix: RoleMembers storage tier
//!
//! Issue #345 noted that `RoleMembers` was documented as "persistent" but
//! stored in `instance()`. Both copies of the bug are fixed here: the index
//! is written to `instance()` storage, which is the correct tier for a
//! bounded, contract-lifetime membership list that must survive any ledger
//! within the contract's active TTL (matching `Role(RoleKey)` and
//! `AdminCount`). If future growth makes the list unbounded, migrate to
//! `persistent()` with explicit TTL management.

use soroban_sdk::{Address, Env, IntoVal, TryFromVal, Val, Vec as SorobanVec};

// ── Trait alias helpers ────────────────────────────────────────────────────

/// Convenience bound for any type usable as an instance-storage key.
///
/// Every `#[contracttype]` enum/struct satisfies this automatically.
pub trait StorageKey: IntoVal<Env, Val> + TryFromVal<Env, Val> + Clone {}

impl<T: IntoVal<Env, Val> + TryFromVal<Env, Val> + Clone> StorageKey for T {}

// ── Error type ────────────────────────────────────────────────────────────

/// Errors that the shared RBAC helpers can return.
///
/// Each contract maps these to its own `Error` enum via `From` or a match
/// arm, so no `contracterror` attribute is needed here.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RbacError {
    /// The caller does not hold the required role or `Admin`.
    NotAuthorized,
    /// Refused to revoke the last `Admin`, which would freeze governance.
    LastAdmin,
}

// ── Core helpers ──────────────────────────────────────────────────────────

/// Whether `account` currently holds the role identified by `role_key`.
///
/// `role_key` is the composite `(role, account)` key used to test membership
/// — typically `DataKey::Role(RoleKey { role, account })`.
pub fn has_role<RK: StorageKey>(env: &Env, role_key: &RK) -> bool {
    env.storage().instance().has(role_key)
}

/// Number of accounts currently holding `Admin` (zero pre-initialization).
///
/// `admin_count_key` is the key under which the count is stored — typically
/// `DataKey::AdminCount`.
pub fn admin_count<AK: StorageKey>(env: &Env, admin_count_key: &AK) -> u32 {
    env.storage()
        .instance()
        .get(admin_count_key)
        .unwrap_or(0u32)
}

/// Grants the role identified by `role_key` to an account.
///
/// - `role_key`:       composite `(role, account)` key
/// - `admin_count_key`: key for the admin counter (incremented when `is_admin == true`)
/// - `members_key`:    key for the `Vec<Address>` membership index
/// - `is_admin`:       `true` when the role being granted is `Admin`
/// - `account`:        the address being granted the role
///
/// Idempotent: re-granting a held role is a no-op.
/// Returns `true` if the role was newly granted, `false` if already held.
pub fn grant<RK, AK, MK>(
    env: &Env,
    role_key: &RK,
    admin_count_key: &AK,
    members_key: &MK,
    is_admin: bool,
    account: &Address,
) -> bool
where
    RK: StorageKey,
    AK: StorageKey,
    MK: StorageKey,
{
    if has_role(env, role_key) {
        return false;
    }
    env.storage().instance().set(role_key, &true);
    if is_admin {
        let next = admin_count(env, admin_count_key) + 1;
        env.storage().instance().set(admin_count_key, &next);
    }
    // Maintain the role-members index.
    let mut members: SorobanVec<Address> = env
        .storage()
        .instance()
        .get(members_key)
        .unwrap_or(SorobanVec::new(env));
    members.push_back(account.clone());
    env.storage().instance().set(members_key, &members);
    true
}

/// Revokes the role identified by `role_key` from an account.
///
/// - `role_key`:        composite `(role, account)` key
/// - `admin_count_key`: key for the admin counter (decremented when `is_admin == true`)
/// - `members_key`:     key for the `Vec<Address>` membership index
/// - `is_admin`:        `true` when the role being revoked is `Admin`
/// - `account`:         the address whose role is being revoked
///
/// Idempotent: revoking a role not held returns `Ok(false)`.
/// Refuses to revoke the last Admin (`Err(RbacError::LastAdmin)`).
/// Returns `Ok(true)` when the role was actually removed.
///
/// **Note:** Oracle-specific side-effects (e.g. removing a `PriceFeeder`
/// from the submitters set) belong at the call site, gated on the
/// `Ok(true)` return value.
pub fn revoke<RK, AK, MK>(
    env: &Env,
    role_key: &RK,
    admin_count_key: &AK,
    members_key: &MK,
    is_admin: bool,
    account: &Address,
) -> Result<bool, RbacError>
where
    RK: StorageKey,
    AK: StorageKey,
    MK: StorageKey,
{
    if !has_role(env, role_key) {
        return Ok(false);
    }
    if is_admin {
        let count = admin_count(env, admin_count_key);
        if count <= 1 {
            return Err(RbacError::LastAdmin);
        }
        env.storage().instance().set(admin_count_key, &(count - 1));
    }
    env.storage().instance().remove(role_key);
    // Rebuild the members index without this account.
    let members: SorobanVec<Address> = env
        .storage()
        .instance()
        .get(members_key)
        .unwrap_or(SorobanVec::new(env));
    let mut updated = SorobanVec::new(env);
    for i in 0..members.len() {
        let m = members.get(i).unwrap();
        if m != *account {
            updated.push_back(m);
        }
    }
    env.storage().instance().set(members_key, &updated);
    Ok(true)
}

/// Returns every account currently holding a role.
///
/// `members_key` is the `DataKey::RoleMembers(role)` variant maintained by
/// `grant` and `revoke`. Returns an empty vector when no accounts hold the role.
pub fn role_members<MK: StorageKey>(env: &Env, members_key: &MK) -> SorobanVec<Address> {
    env.storage()
        .instance()
        .get(members_key)
        .unwrap_or(SorobanVec::new(env))
}

/// Requires that `caller` authorized the transaction and holds the role
/// identified by `role_key` **or** is an `Admin` (identified by `admin_key`).
///
/// - `caller`:    the signer being checked
/// - `role_key`:  composite `(role, caller)` key for the specific role
/// - `admin_key`: composite `(Admin, caller)` key for the Admin super-user check
/// - `on_success`: optional callback invoked after a successful auth check,
///   used by `DripGovernor` to bump instance TTL. Pass `None` from contexts
///   where TTL is managed at the entry-point level (e.g. `DripOracle`).
///
/// Returns `Ok(())` on success, `Err(RbacError::NotAuthorized)` otherwise.
pub fn require_role_or_admin<RK: StorageKey>(
    env: &Env,
    caller: &Address,
    role_key: &RK,
    admin_key: &RK,
    on_success: Option<fn(&Env)>,
) -> Result<(), RbacError> {
    caller.require_auth();
    if has_role(env, admin_key) || has_role(env, role_key) {
        if let Some(bump) = on_success {
            bump(env);
        }
        Ok(())
    } else {
        Err(RbacError::NotAuthorized)
    }
}

/// Requires that `caller` authorized the transaction and holds the role
/// identified by `role_key` exactly (no Admin fallback).
///
/// Used by `DripGovernor::require_role` for operations that must be
/// performed by the exact role holder — e.g. Admin-only `grant_role`.
/// Pass `on_success` to bump TTL on success.
///
/// Returns `Ok(())` on success, `Err(RbacError::NotAuthorized)` otherwise.
pub fn require_role<RK: StorageKey>(
    env: &Env,
    caller: &Address,
    role_key: &RK,
    on_success: Option<fn(&Env)>,
) -> Result<(), RbacError> {
    caller.require_auth();
    if has_role(env, role_key) {
        if let Some(bump) = on_success {
            bump(env);
        }
        Ok(())
    } else {
        Err(RbacError::NotAuthorized)
    }
}
