#![no_std]
#![allow(deprecated)]

//! DripGovernor is the protocol's immutable parameter store.
//!
//! **Storage and TTL:** All state is held in `instance()` storage tied to the
//! contract's instance TTL. Every state-mutating call (`grant_role`, `revoke_role`,
//! `transfer_authority`, `set_fee_bps`, `set_fee_recipient`, `set_min_duration`,
//! `set_max_rate`, `set_max_duration`) extends the instance TTL to prevent
//! archival. Without TTL extension, idle governors can expire and cause all
//! downstream `DripFactory::create_stream` calls to fail until restored.
//!
//! **Roles:** Four role tiers allow delegation of governance authority:
//! - `Admin`: grant and revoke any role (super-user).
//! - `FeeManager`: manage fees via `set_fee_bps` and `set_fee_recipient`.
//! - `RateManager`: manage rate and duration bounds via `set_max_rate`, `set_min_duration`, `set_max_duration`.
//! - `Pauser`: emergency pause/unpause via `governor_pause`, `governor_unpause`, `pause_factory`, `unpause_factory`.

mod config;
mod errors;
mod events;
mod role;
mod storage;
mod tests;
mod ttl;

use soroban_sdk::{contract, contractimpl, panic_with_error, Address, BytesN, Env, Symbol, Vec};

use drip_common::is_zero_address;

pub use config::GovernorConfig;
pub use errors::Error;
pub use role::Role;
use storage::DataKey;

/// Returns `true` when the governor is under an emergency pause.
///
/// Defaults to `false` when the key has never been set — e.g. a governor
/// that was initialized before this feature existed. This keeps the flag
/// backward-compatible: an absent entry means "running normally".
fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false)
}

/// Assert that the governor is not paused. Returns `ContractPaused` if it is.
fn assert_not_paused(env: &Env) -> Result<(), Error> {
    if is_paused(env) {
        Err(Error::ContractPaused)
    } else {
        Ok(())
    }
}

#[contract]
pub struct DripGovernor;

#[contractimpl]
#[allow(deprecated)]
impl DripGovernor {
    /// Deploy-time initialisation.
    ///
    /// Guards against re-initialization: without this check, anyone could call
    /// `initialize` again to grant themselves `Admin`, then set `fee_bps` to
    /// the maximum or repoint `fee_recipient`.
    ///
    /// The deploy `authority` is granted every role, so a single wallet can
    /// bootstrap the protocol and later delegate fee and rate/duration
    /// management to separate wallets via [`DripGovernor::grant_role`].
    pub fn initialize(
        env: Env,
        authority: Address,
        fee_recipient: Address,
        factory_address: Address,
    ) {
        if env.storage().instance().has(&DataKey::FactoryAddress) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        ttl::bump(&env);

        let s = env.storage().instance();
        s.set(&DataKey::FeeBps, &30_u32);
        s.set(&DataKey::FeeRecipient, &fee_recipient);
        s.set(&DataKey::MinDurationSeconds, &3600_u64);
        s.set(&DataKey::MaxDurationSeconds, &315_360_000_u64); // 10 years
        s.set(&DataKey::MaxRatePerSecond, &1_000_000_000_000_000_i128);
        s.set(&DataKey::FactoryAddress, &factory_address);
        s.set(&DataKey::ForceCancelPauseThresholdSeconds, &2_592_000_u64); // 30 days

        role::grant(&env, Role::Admin, &authority);
        role::grant(&env, Role::FeeManager, &authority);
        role::grant(&env, Role::RateManager, &authority);
        role::grant(&env, Role::Pauser, &authority);
        events::initialized(&env, &authority, &fee_recipient, &factory_address);
    }

    // ── Reads ────────────────────────────────────────────────────────────

    /// Returns the complete [`GovernorConfig`] from storage.
    ///
    /// Returns `Err(NotInitialized)` if the governor has not yet been initialized.
    pub fn config(env: Env) -> Result<GovernorConfig, Error> {
        config::load(&env)
    }

    /// Read-only: current minimum stream duration in seconds.
    ///
    /// Focused accessor for callers that only need this one field, avoiding
    /// a full `config()` round-trip (mirrors `DripFactory::protocol_fee_bps`).
    ///
    /// Returns `Err(NotInitialized)` for an uninitialised governor, matching
    /// [`DripGovernor::max_duration`], [`DripGovernor::max_rate`], and
    /// [`DripGovernor::config`] — previously this accessor alone silently
    /// returned a hardcoded `3600` default, so callers could not distinguish
    /// "governor says 3600" from "governor doesn't exist yet".
    pub fn min_duration(env: Env) -> Result<u64, Error> {
        Ok(config::load(&env)?.min_duration_seconds)
    }

    /// Whether `account` currently holds `role`.
    pub fn has_role(env: Env, role: Role, account: Address) -> bool {
        role::has_role(&env, role, &account)
    }

    /// Returns every account currently holding `role`.
    ///
    /// Enables on-chain role-membership auditing without replaying every
    /// `grant_role`/`revoke_role` event from genesis. The index is maintained
    /// automatically by `grant_role` and `revoke_role`.
    pub fn role_members(env: Env, role: Role) -> Vec<Address> {
        role::role_members(&env, role)
    }

    /// Current maximum stream duration in seconds, without fetching the full
    /// [`GovernorConfig`]. See [`DripGovernor::set_max_duration`].
    pub fn max_duration(env: Env) -> Result<u64, Error> {
        Ok(config::load(&env)?.max_duration_seconds)
    }

    /// Current maximum rate per second a stream may commit to, without
    /// fetching the full [`GovernorConfig`]. See [`DripGovernor::set_max_rate`].
    pub fn max_rate(env: Env) -> Result<i128, Error> {
        Ok(config::load(&env)?.max_rate_per_second)
    }

    /// Read-only: whether the governor is currently under an emergency pause.
    ///
    /// Named `governor_is_paused` (not `is_paused`) because `drip-factory`
    /// depends on this crate directly (for `DripGovernorClient` /
    /// `GovernorConfig`), which pulls this contract's compiled code into
    /// factory's own WASM link step. `DripFactory` already has its own
    /// `is_paused`/`pause`/`unpause` methods; using the same names here
    /// causes a hard `duplicate symbol` link error in `drip_factory.wasm`.
    pub fn governor_is_paused(env: Env) -> bool {
        is_paused(&env)
    }

    // ── Emergency pause (Admin-gated) ────────────────────────────────────

    /// Emergency halt: freeze all parameter writes.
    ///
    /// While paused, every `set_*` method reverts with `ContractPaused`
    /// before any state is touched. Role administration (`grant_role`,
    /// `revoke_role`, `transfer_authority`) remains operational so the
    /// admin can still manage access during an emergency.
    ///
    /// Named `governor_pause` — see `governor_is_paused` for why.
    pub fn governor_pause(env: Env, caller: Address) -> Result<(), Error> {
        role::require_role_or_admin(&env, &caller, Role::Pauser)?;
        if is_paused(&env) {
            return Err(Error::AlreadyPaused);
        }
        env.storage().instance().set(&DataKey::Paused, &true);
        events::paused(&env, &caller, env.ledger().timestamp());
        Ok(())
    }

    /// Lift the emergency pause, allowing parameter writes again.
    ///
    /// Named `governor_unpause` — see `governor_is_paused` for why.
    pub fn governor_unpause(env: Env, caller: Address) -> Result<(), Error> {
        role::require_role_or_admin(&env, &caller, Role::Pauser)?;
        if !is_paused(&env) {
            return Err(Error::NotPaused);
        }
        env.storage().instance().set(&DataKey::Paused, &false);
        events::unpaused(&env, &caller, env.ledger().timestamp());
        Ok(())
    }

    // ── Factory pause passthrough (Admin-gated) ──────────────────────────
    //
    // `DripFactory::pause`/`unpause` are gated on `governor.require_auth()`,
    // where `governor` is the address the factory was initialized with. When
    // that address is this `DripGovernor` contract — the wiring the rest of
    // the codebase (`governance::config`/`enforce_bounds` in `drip-factory`)
    // already assumes — the factory has no entry point reachable through the
    // role system without a passthrough like this: a direct EOA call to
    // `factory.pause()` can never satisfy `governor.require_auth()` unless
    // the EOA *is* `governor`. A contract, on the other hand, implicitly
    // satisfies `require_auth()` for its own address when it is the direct
    // invoker of the call, so this cross-contract call succeeds where a
    // plain account call could not.
    //
    // `drip-governor` cannot depend on the `drip-factory` crate (factory
    // already depends on governor — a reverse dependency would be
    // circular), so the call goes through `env.try_invoke_contract` by
    // function name instead of a generated client.

    /// Role-gated passthrough: pauses the `DripFactory` this governor
    /// controls, so a `Role::Admin` or `Role::Pauser` holder has an actual
    /// entry point to trigger the factory's emergency halt.
    pub fn pause_factory(env: Env, caller: Address) -> Result<(), Error> {
        role::require_role_or_admin(&env, &caller, Role::Pauser)?;
        let factory: Address = env
            .storage()
            .instance()
            .get(&DataKey::FactoryAddress)
            .ok_or(Error::NotInitialized)?;

        let result: Result<
            Result<(), soroban_sdk::ConversionError>,
            Result<soroban_sdk::Error, soroban_sdk::InvokeError>,
        > = env.try_invoke_contract(&factory, &Symbol::new(&env, "pause"), Vec::new(&env));
        match result {
            Ok(Ok(())) => {
                events::factory_paused(&env, &caller, env.ledger().timestamp());
                Ok(())
            }
            _ => Err(Error::FactoryCallFailed),
        }
    }

    /// Role-gated passthrough: lifts the `DripFactory` emergency pause. See
    /// `pause_factory`.
    pub fn unpause_factory(env: Env, caller: Address) -> Result<(), Error> {
        role::require_role_or_admin(&env, &caller, Role::Pauser)?;
        let factory: Address = env
            .storage()
            .instance()
            .get(&DataKey::FactoryAddress)
            .ok_or(Error::NotInitialized)?;

        let result: Result<
            Result<(), soroban_sdk::ConversionError>,
            Result<soroban_sdk::Error, soroban_sdk::InvokeError>,
        > = env.try_invoke_contract(&factory, &Symbol::new(&env, "unpause"), Vec::new(&env));
        match result {
            Ok(Ok(())) => {
                events::factory_unpaused(&env, &caller, env.ledger().timestamp());
                Ok(())
            }
            _ => Err(Error::FactoryCallFailed),
        }
    }

    // ── Role administration (Admin-gated) ────────────────────────────────

    /// Grants `role` to `account`. Only an `Admin` may call this.
    pub fn grant_role(
        env: Env,
        caller: Address,
        role: Role,
        account: Address,
    ) -> Result<(), Error> {
        role::require_role(&env, &caller, Role::Admin)?;
        if role::grant(&env, role, &account) {
            events::grant_role(&env, &caller, role, &account);
        }
        Ok(())
    }

    /// Revokes `role` from `account`. Only an `Admin` may call this.
    ///
    /// Idempotent: revoking a role from an account that never held it (or has
    /// already been revoked) is a silent no-op. No error is raised.
    ///
    /// Rejected with `LastAdmin` if it would remove the final `Admin`.
    pub fn revoke_role(
        env: Env,
        caller: Address,
        role: Role,
        account: Address,
    ) -> Result<(), Error> {
        role::require_role(&env, &caller, Role::Admin)?;
        if role::revoke(&env, role, &account)? {
            events::revoke_role(&env, &caller, role, &account);
        }
        Ok(())
    }

    /// Hands the full `Admin` role from `caller` to `new_authority`.
    ///
    /// Grants first so the subsequent revoke can never trip the `LastAdmin`
    /// guard, then revokes `caller`. Kept for API familiarity — equivalent to
    /// a `grant_role(Admin, new)` followed by `revoke_role(Admin, caller)`.
    ///
    /// **Deprecated**: Use `propose_authority` + `accept_authority` instead for
    /// the safer 2-step transfer pattern (Ownable2Step).
    #[deprecated(
        note = "Use `propose_authority` + `accept_authority` instead for the safer 2-step transfer pattern (Ownable2Step)."
    )]
    pub fn transfer_authority(
        env: Env,
        caller: Address,
        new_authority: Address,
    ) -> Result<(), Error> {
        if is_zero_address(&env, &new_authority)
            || role::has_role(&env, Role::Admin, &new_authority)
        {
            return Err(Error::InvalidParam);
        }
        role::require_role(&env, &caller, Role::Admin)?;
        role::grant(&env, Role::Admin, &new_authority);
        role::revoke(&env, Role::Admin, &caller)?;
        events::transfer_authority(&env, &caller, &new_authority);
        Ok(())
    }

    /// Propose a new authority address (step 1 of 2).
    ///
    /// Stores the pending authority address and the proposer. The transfer is
    /// not complete until the pending authority calls `accept_authority`. This
    /// ensures the new address is live and can actually sign transactions
    /// before the handoff.
    pub fn propose_authority(
        env: Env,
        caller: Address,
        new_authority: Address,
    ) -> Result<(), Error> {
        if is_zero_address(&env, &new_authority)
            || role::has_role(&env, Role::Admin, &new_authority)
        {
            return Err(Error::InvalidParam);
        }
        role::require_role(&env, &caller, Role::Admin)?;
        env.storage()
            .instance()
            .set(&DataKey::PendingAuthority, &new_authority);
        env.storage()
            .instance()
            .set(&DataKey::PendingAuthorityProposer, &caller);
        events::propose_authority(&env, &caller, &new_authority);
        Ok(())
    }

    /// Accept the pending authority transfer (step 2 of 2).
    ///
    /// Must be called by the pending authority address itself. This completes
    /// the transfer: the pending authority gains `Admin` and the proposer
    /// loses it.
    pub fn accept_authority(env: Env, caller: Address) -> Result<(), Error> {
        let pending: Address = env
            .storage()
            .instance()
            .get(&DataKey::PendingAuthority)
            .ok_or(Error::NoPendingAuthority)?;
        if caller != pending {
            return Err(Error::NotPendingAuthority);
        }
        caller.require_auth();
        ttl::bump(&env);

        // Grant Admin to the new authority (the caller) first, then revoke from proposer.
        // This ensures the LastAdmin guard never blocks the transfer.
        let proposer: Address = env
            .storage()
            .instance()
            .get(&DataKey::PendingAuthorityProposer)
            .ok_or(Error::NoPendingAuthority)?;
        role::grant(&env, Role::Admin, &caller);
        let _ = role::revoke(&env, Role::Admin, &proposer)?;

        // Clean up pending state.
        env.storage().instance().remove(&DataKey::PendingAuthority);
        env.storage()
            .instance()
            .remove(&DataKey::PendingAuthorityProposer);

        events::accept_authority(&env, &caller, &proposer);
        Ok(())
    }

    // ── Self-upgrade (Admin-gated) ──────────────────────────────────────

    /// Replace this contract's own WASM bytecode.
    ///
    /// The new WASM must already be uploaded to the ledger (via
    /// `stellar contract upload`); only the hash is passed here. Gated on
    /// `Admin` so a compromised or abandoned admin key cannot silently swap
    /// the implementation.
    ///
    /// Blocked while the governor is paused, matching the factory's
    /// `upgrade` guard — a halted protocol should accept no code changes.
    pub fn upgrade(env: Env, caller: Address, new_wasm_hash: BytesN<32>) -> Result<(), Error> {
        role::require_role(&env, &caller, Role::Admin)?;
        assert_not_paused(&env)?;

        if new_wasm_hash == BytesN::from_array(&env, &[0u8; 32]) {
            return Err(Error::InvalidWasmHash);
        }

        ttl::bump(&env);
        env.deployer().update_current_contract_wasm(new_wasm_hash);
        events::upgraded(&env, &caller, env.ledger().timestamp());
        Ok(())
    }

    // ── Parameter writes (role-gated + pause guard) ──────────────────────

    /// Sets the protocol fee in basis points (1 bps = 0.01%).
    ///
    /// Only a `FeeManager` (or `Admin`) may call this. The fee must be between
    /// 0 and 10,000 basis points (0% to 100%); values above 10,000 revert with
    /// `Error::InvalidParam`.
    pub fn set_fee_bps(env: Env, caller: Address, fee_bps: u32) -> Result<(), Error> {
        if fee_bps > 10_000 {
            return Err(Error::InvalidParam);
        }
        assert_not_paused(&env)?;
        role::require_role_or_admin(&env, &caller, Role::FeeManager)?;
        let old_fee_bps: u32 = env.storage().instance().get(&DataKey::FeeBps).unwrap_or(30);
        env.storage().instance().set(&DataKey::FeeBps, &fee_bps);
        events::set_fee_bps(&env, &caller, old_fee_bps, fee_bps);
        Ok(())
    }

    pub fn set_fee_recipient(env: Env, caller: Address, recipient: Address) -> Result<(), Error> {
        if is_zero_address(&env, &recipient) {
            return Err(Error::InvalidParam);
        }
        assert_not_paused(&env)?;
        role::require_role_or_admin(&env, &caller, Role::FeeManager)?;
        let old_recipient: Address = env
            .storage()
            .instance()
            .get(&DataKey::FeeRecipient)
            .ok_or(Error::NotInitialized)?;
        env.storage()
            .instance()
            .set(&DataKey::FeeRecipient, &recipient);
        events::set_fee_recipient(&env, &caller, &old_recipient, &recipient);
        Ok(())
    }

    pub fn set_min_duration(env: Env, caller: Address, seconds: u64) -> Result<(), Error> {
        if seconds == 0 {
            return Err(Error::InvalidParam);
        }
        assert_not_paused(&env)?;
        role::require_role_or_admin(&env, &caller, Role::RateManager)?;
        let old_min_duration: u64 = env
            .storage()
            .instance()
            .get(&DataKey::MinDurationSeconds)
            .unwrap_or(3600);
        let max_duration: u64 = env
            .storage()
            .instance()
            .get(&DataKey::MaxDurationSeconds)
            .unwrap_or(315_360_000);
        if seconds > max_duration {
            return Err(Error::InvalidParam);
        }
        let max_rate: i128 = env
            .storage()
            .instance()
            .get(&DataKey::MaxRatePerSecond)
            .unwrap_or(1_000_000_000_000_000);
        let secs_i: i128 = seconds as i128;
        max_rate.checked_mul(secs_i).ok_or(Error::InvalidParam)?;
        env.storage()
            .instance()
            .set(&DataKey::MinDurationSeconds, &seconds);
        events::set_min_duration(&env, &caller, old_min_duration, seconds);
        Ok(())
    }

    pub fn set_max_rate(env: Env, caller: Address, max_rate: i128) -> Result<(), Error> {
        if max_rate <= 0 {
            return Err(Error::InvalidParam);
        }
        assert_not_paused(&env)?;
        role::require_role_or_admin(&env, &caller, Role::RateManager)?;
        let old_max_rate: i128 = env
            .storage()
            .instance()
            .get(&DataKey::MaxRatePerSecond)
            .unwrap_or(1_000_000_000_000_000);
        let min_duration: u64 = env
            .storage()
            .instance()
            .get(&DataKey::MinDurationSeconds)
            .unwrap_or(3_600);
        let secs_i: i128 = min_duration as i128;
        max_rate.checked_mul(secs_i).ok_or(Error::InvalidParam)?;
        env.storage()
            .instance()
            .set(&DataKey::MaxRatePerSecond, &max_rate);
        events::set_max_rate(&env, &caller, old_max_rate, max_rate);
        Ok(())
    }

    pub fn set_max_duration(env: Env, caller: Address, seconds: u64) -> Result<(), Error> {
        if seconds == 0 {
            return Err(Error::InvalidParam);
        }
        assert_not_paused(&env)?;
        role::require_role_or_admin(&env, &caller, Role::RateManager)?;
        let old_max_duration: u64 = env
            .storage()
            .instance()
            .get(&DataKey::MaxDurationSeconds)
            .unwrap_or(315_360_000);
        let min_duration: u64 = env
            .storage()
            .instance()
            .get(&DataKey::MinDurationSeconds)
            .unwrap_or(3_600);
        if seconds < min_duration {
            return Err(Error::InvalidParam);
        }
        env.storage()
            .instance()
            .set(&DataKey::MaxDurationSeconds, &seconds);
        events::set_max_duration(&env, &caller, old_max_duration, seconds);
        Ok(())
    }

    /// Sets the number of seconds a stream must remain paused before its
    /// recipient may call `DripStream::force_cancel`.
    ///
    /// Governance-configurable per deployment: a payroll protocol might want
    /// 7 days, a long-horizon vesting stream 90. Gated by `RateManager` —
    /// the same role tier that already controls the other stream bounds
    /// (`set_min_duration`, `set_max_duration`, `set_max_rate`).
    pub fn set_force_cancel_pause_threshold(
        env: Env,
        caller: Address,
        seconds: u64,
    ) -> Result<(), Error> {
        if seconds == 0 {
            return Err(Error::InvalidParam);
        }
        assert_not_paused(&env)?;
        role::require_role_or_admin(&env, &caller, Role::RateManager)?;
        env.storage()
            .instance()
            .set(&DataKey::ForceCancelPauseThresholdSeconds, &seconds);
        events::set_force_cancel_pause_threshold(&env, &caller, seconds);
        Ok(())
    }
}
