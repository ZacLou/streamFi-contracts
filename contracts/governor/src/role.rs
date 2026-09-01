use soroban_sdk::{Address, Env, Vec as SorobanVec};

use drip_common::rbac;

use crate::storage::{DataKey, RoleKey};
use crate::ttl;
use crate::Error;

/// Re-export so callers using `role::Role` continue to work unchanged.
pub use crate::storage::Role;

// ── Key helpers ────────────────────────────────────────────────────────────

fn role_key(role: Role, account: &Address) -> DataKey {
    DataKey::Role(RoleKey {
        role,
        account: account.clone(),
    })
}

fn members_key(role: Role) -> DataKey {
    DataKey::RoleMembers(role)
}

// ── Public API (delegates to drip_common::rbac) ────────────────────────────

/// Whether `account` currently holds `role`.
pub fn has_role(env: &Env, role: Role, account: &Address) -> bool {
    rbac::has_role(env, &role_key(role, account))
}

/// Number of accounts currently holding `Role::Admin` (zero pre-initialization).
///
/// Part of the governor RBAC surface; no contract entry point calls it yet.
#[allow(dead_code)]
pub fn admin_count(env: &Env) -> u32 {
    rbac::admin_count(env, &DataKey::AdminCount)
}

/// Grants `role` to `account`.
///
/// Idempotent: re-granting a role the account already holds is a no-op, so the
/// admin count can never be inflated by repeated grants. Returns `true` if the
/// role was newly granted, or `false` if the account already held it.
pub fn grant(env: &Env, role: Role, account: &Address) -> bool {
    rbac::grant(
        env,
        &role_key(role, account),
        &DataKey::AdminCount,
        &members_key(role),
        role == Role::Admin,
        account,
    )
}

/// Revokes `role` from `account`.
///
/// Idempotent when the account doesn't hold the role. Refuses to remove the
/// final `Admin` (`LastAdmin`): a governor with zero admins could never grant
/// a new one, permanently freezing every protocol parameter. Returns `true` if
/// the role was newly revoked, or `false` if the account did not hold it.
pub fn revoke(env: &Env, role: Role, account: &Address) -> Result<bool, Error> {
    rbac::revoke(
        env,
        &role_key(role, account),
        &DataKey::AdminCount,
        &members_key(role),
        role == Role::Admin,
        account,
    )
    .map_err(|e| match e {
        rbac::RbacError::LastAdmin => Error::LastAdmin,
        rbac::RbacError::NotAuthorized => Error::NotAuthorized,
    })
}

/// Requires that `caller` both authorized the transaction and holds `role`,
/// then bumps instance TTL. Every role-gated write funnels through here.
pub fn require_role(env: &Env, caller: &Address, role: Role) -> Result<(), Error> {
    rbac::require_role(env, caller, &role_key(role, caller), Some(ttl::bump))
        .map_err(|_| Error::NotAuthorized)
}

/// Returns every account currently holding `role`.
///
/// Reads from the `RoleMembers` index maintained by `grant`/`revoke`.
/// Returns an empty vector if no accounts hold the role.
pub fn role_members(env: &Env, role: Role) -> SorobanVec<Address> {
    rbac::role_members(env, &members_key(role))
}

/// Requires that `caller` authorized the transaction and holds `role` **or**
/// is an `Admin`.  Admin acts as a super-user: even after delegating
/// `FeeManager` / `RateManager` to dedicated wallets the deployer can still
/// adjust every parameter directly.
///
/// Bumps instance TTL on success.
pub fn require_role_or_admin(env: &Env, caller: &Address, role: Role) -> Result<(), Error> {
    rbac::require_role_or_admin(
        env,
        caller,
        &role_key(role, caller),
        &role_key(Role::Admin, caller),
        Some(ttl::bump),
    )
    .map_err(|_| Error::NotAuthorized)
}
