#![no_std]

mod errors;
mod events;
mod storage;
#[cfg(test)]
mod tests;

use drip_common::{is_zero_address, TTL_EXTEND_TO, TTL_THRESHOLD};
use errors::Error;
use soroban_sdk::{contract, contractimpl, token, Address, Env};
use storage::{
    get_max_limit, get_operator, get_operator_withdraw_limit, get_owner, get_pending_owner,
    get_pending_owner_proposer, get_token, is_paused, remove_operator, remove_pending_owner,
    remove_pending_owner_proposer, set_max_limit, set_operator, set_operator_withdraw_limit,
    set_owner, set_paused, set_pending_owner, set_pending_owner_proposer, set_token,
};

#[contract]
pub struct TokenVault;

/// Extends the instance storage TTL to ensure vault state remains active and
/// does not archive during idle periods. Matches the TTL management pattern
/// across sibling contracts (DripFactory, DripGovernor, TwapOracle).
fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
}

fn token_client(env: &Env) -> Result<token::Client<'_>, Error> {
    let token_addr = get_token(env).ok_or(Error::NotInitialized)?;
    Ok(token::Client::new(env, &token_addr))
}

fn vault_balance(env: &Env) -> Result<i128, Error> {
    Ok(token_client(env)?.balance(&env.current_contract_address()))
}

/// Checks that `caller` is either the vault owner or the currently delegated
/// operator, then consumes the caller's auth. Returns `NotAuthorized` if
/// `caller` matches neither role.
///
/// Mirrors `DripStream::require_sender_or_operator` — the owner can hand off
/// day-to-day withdrawal authority to a hot wallet / ops key without exposing
/// the cold owner key for routine operations.
fn require_owner_or_operator(env: &Env, caller: &Address, owner: &Address) -> Result<(), Error> {
    let operator = get_operator(env);
    let is_owner = caller == owner;
    let is_op = operator.as_ref().map(|op| caller == op).unwrap_or(false);
    if is_owner || is_op {
        caller.require_auth();
        Ok(())
    } else {
        Err(Error::NotAuthorized)
    }
}

/// Short-circuit helper: reject any state-mutating call while paused.
fn assert_not_paused(env: &Env) -> Result<(), Error> {
    if is_paused(env) {
        Err(Error::ContractPaused)
    } else {
        Ok(())
    }
}

#[contractimpl]
impl TokenVault {
    /// Initialise the vault.
    ///
    /// # Errors
    ///
    /// - `InvalidAmount` — `max_limit` is not positive.
    /// - `AlreadyInitialized` — the vault already has an owner.
    ///
    /// Returns `Result` rather than panicking so the typed error reaches the
    /// caller through `try_initialize`. A `panic_with_error!` surfaces only as
    /// an untyped host error, which means a caller cannot distinguish
    /// `AlreadyInitialized` from any other trap without matching on a raw
    /// numeric code. `DripOracle::initialize` already returns `Result` for the
    /// same reason.
    pub fn initialize(
        env: Env,
        owner: Address,
        token: Address,
        max_limit: i128,
    ) -> Result<(), Error> {
        owner.require_auth();

        if max_limit <= 0 {
            return Err(Error::InvalidAmount);
        }

        if get_owner(&env).is_some() {
            return Err(Error::AlreadyInitialized);
        }

        bump_instance(&env);
        set_owner(&env, &owner);
        set_token(&env, &token);
        set_max_limit(&env, &max_limit);

        events::initialized(&env, &owner, &token, max_limit);
        Ok(())
    }

    pub fn deposit(env: Env, from: Address, amount: i128) -> Result<(), Error> {
        assert_not_paused(&env)?;
        let owner = get_owner(&env).ok_or(Error::NotInitialized)?;
        require_owner_or_operator(&env, &from, &owner)?;

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let _owner = get_owner(&env).ok_or(Error::NotInitialized)?;
        let balance = vault_balance(&env)?;
        let max = get_max_limit(&env).ok_or(Error::NotInitialized)?;

        let expected_balance = balance
            .checked_add(amount)
            .ok_or(Error::ArithmeticOverflow)?;
        if expected_balance > max {
            return Err(Error::LimitExceeded);
        }

        let tk = token_client(&env)?;
        tk.transfer(&from, &env.current_contract_address(), &amount);
        let new_balance = tk.balance(&env.current_contract_address());
        if new_balance > max {
            return Err(Error::LimitExceeded);
        }
        if new_balance != expected_balance {
            return Err(Error::DepositTransferFailed);
        }

        bump_instance(&env);
        events::deposited(&env, &from, amount, new_balance);
        Ok(())
    }

    pub fn withdraw(env: Env, caller: Address, to: Address, amount: i128) -> Result<(), Error> {
        assert_not_paused(&env)?;
        let owner = get_owner(&env).ok_or(Error::NotInitialized)?;
        require_owner_or_operator(&env, &caller, &owner)?;

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        if caller != owner {
            let limit = get_operator_withdraw_limit(&env).ok_or(Error::LimitExceeded)?;
            if amount > limit {
                return Err(Error::LimitExceeded);
            }
        }

        let balance = vault_balance(&env)?;
        let expected_balance = balance
            .checked_sub(amount)
            .ok_or(Error::ArithmeticOverflow)?;

        let tk = token_client(&env)?;
        tk.transfer(&env.current_contract_address(), &to, &amount);
        let new_balance = vault_balance(&env)?;
        if new_balance != expected_balance {
            return Err(Error::DepositTransferFailed);
        }

        bump_instance(&env);
        events::withdrawn(&env, &caller, &to, amount, new_balance);
        Ok(())
    }

    /// Raising `max_limit` requires `caller == owner`; an operator may only
    /// lower it. `max_limit` is the vault's core risk parameter — it caps
    /// total exposure — so a delegated operator key (a hot wallet meant for
    /// day-to-day operations) must not be able to expand it. This matches
    /// the general principle that delegated keys can reduce but not expand
    /// authority; operators can still tighten the cap on their own.
    pub fn set_limit(env: Env, caller: Address, new_limit: i128) -> Result<(), Error> {
        assert_not_paused(&env)?;
        let owner = get_owner(&env).ok_or(Error::NotInitialized)?;

        if new_limit <= 0 {
            return Err(Error::InvalidAmount);
        }
        let balance = vault_balance(&env)?;
        if new_limit < balance {
            return Err(Error::LimitExceeded);
        }
        let old_limit = get_max_limit(&env).ok_or(Error::ArithmeticOverflow)?;

        if new_limit > old_limit {
            if caller != owner {
                return Err(Error::NotAuthorized);
            }
            caller.require_auth();
        } else {
            require_owner_or_operator(&env, &caller, &owner)?;
        }

        bump_instance(&env);
        set_max_limit(&env, &new_limit);
        events::limit_set(&env, &caller, old_limit, new_limit);
        Ok(())
    }

    // ── Operator delegation (owner-gated) ─────────────────────────────────

    /// Owner designates an operator who can perform day-to-day actions on
    /// this vault: `withdraw`, and `set_limit` to *lower* (not raise) the
    /// deposit cap. Raising `max_limit` remains owner-only — see
    /// [`TokenVault::set_limit`] — so a compromised operator key cannot
    /// expand the vault's risk exposure.
    ///
    /// Only the owner may call this. Matches `DripStream::set_operator` — the
    /// owner can delegate day-to-day operations to a hot wallet while keeping
    /// the owner key in cold storage.
    pub fn set_operator(env: Env, caller: Address, operator: Address) -> Result<(), Error> {
        let owner = get_owner(&env).ok_or(Error::NotInitialized)?;
        if caller != owner {
            return Err(Error::NotAuthorized);
        }
        caller.require_auth();
        bump_instance(&env);
        set_operator(&env, &operator);
        events::operator_set(&env, &caller, &operator);
        Ok(())
    }

    /// Owner revokes the operator, removing all delegated authority.
    ///
    /// No-op (not an error) if no operator is currently set.
    pub fn revoke_operator(env: Env, caller: Address) -> Result<(), Error> {
        let owner = get_owner(&env).ok_or(Error::NotInitialized)?;
        if caller != owner {
            return Err(Error::NotAuthorized);
        }
        caller.require_auth();
        bump_instance(&env);
        remove_operator(&env);
        events::operator_revoked(&env, &caller);
        Ok(())
    }

    // ── Owner transfer (owner-gated, 2-step) ──────────────────────────────

    /// Propose a new owner (step 1 of 2).
    ///
    /// Only the current owner may call this. The transfer is not complete
    /// until the proposed address calls `accept_owner`. This two-step pattern
    /// (matching `DripGovernor::propose_authority`) allows a lost or
    /// compromised owner key to be rotated out without risking a mistake: the
    /// new address must prove it is live and can actually sign before the old
    /// owner is relinquished.
    ///
    /// # Errors
    ///
    /// - `NotAuthorized` — `caller` is not the current owner.
    /// - `InvalidParam` — `new_owner` is the zero address.
    pub fn propose_owner(env: Env, caller: Address, new_owner: Address) -> Result<(), Error> {
        let owner = get_owner(&env).ok_or(Error::NotInitialized)?;
        if caller != owner {
            return Err(Error::NotAuthorized);
        }
        caller.require_auth();
        if is_zero_address(&env, &new_owner) {
            return Err(Error::InvalidParam);
        }
        bump_instance(&env);
        set_pending_owner(&env, &new_owner);
        set_pending_owner_proposer(&env, &caller);
        events::owner_proposed(&env, &caller, &new_owner);
        Ok(())
    }

    /// Accept the pending owner transfer (step 2 of 2).
    ///
    /// Must be called by the pending owner address itself. Completes the
    /// transfer: the pending owner becomes the vault owner and the previous
    /// owner (the proposer) is removed.
    ///
    /// # Errors
    ///
    /// - `NoPendingOwner` — no owner transfer has been proposed.
    /// - `NotPendingOwner` — `caller` is not the proposed pending owner.
    pub fn accept_owner(env: Env, caller: Address) -> Result<(), Error> {
        let pending = get_pending_owner(&env).ok_or(Error::NoPendingOwner)?;
        if caller != pending {
            return Err(Error::NotPendingOwner);
        }
        caller.require_auth();
        let proposer = get_pending_owner_proposer(&env).ok_or(Error::NoPendingOwner)?;
        bump_instance(&env);
        set_owner(&env, &caller);
        remove_pending_owner(&env);
        remove_pending_owner_proposer(&env);
        events::owner_accepted(&env, &caller, &proposer);
        Ok(())
    }

    /// Read-only: the current operator address, if any.
    pub fn operator(env: Env) -> Option<Address> {
        get_operator(&env)
    }

    /// Read-only: the maximum single-call withdrawal a delegated operator may
    /// execute before the owner raises or removes the cap.
    pub fn operator_withdraw_limit(env: Env) -> Option<i128> {
        get_operator_withdraw_limit(&env)
    }

    /// Read-only: the current owner address, if any.
    pub fn owner(env: Env) -> Option<Address> {
        get_owner(&env)
    }

    // ── Emergency pause (owner-gated) ─────────────────────────────────────

    /// Emergency halt: freeze all state-mutating operations.
    ///
    /// While paused, `deposit`, `withdraw`, and `set_limit` all revert with
    /// `ContractPaused` before touching any state. Matches the
    /// `pause`/`unpause`/`is_paused` triple present on `DripFactory`,
    /// `DripGovernor`, and `TwapOracle`.
    pub fn pause(env: Env, caller: Address) -> Result<(), Error> {
        let owner = get_owner(&env).ok_or(Error::NotInitialized)?;
        if caller != owner {
            return Err(Error::NotAuthorized);
        }
        caller.require_auth();
        if is_paused(&env) {
            return Err(Error::AlreadyPaused);
        }
        bump_instance(&env);
        set_paused(&env, true);
        events::paused(&env, &caller, env.ledger().timestamp());
        Ok(())
    }

    /// Lift the emergency pause, re-enabling all state-mutating operations.
    pub fn unpause(env: Env, caller: Address) -> Result<(), Error> {
        let owner = get_owner(&env).ok_or(Error::NotInitialized)?;
        if caller != owner {
            return Err(Error::NotAuthorized);
        }
        caller.require_auth();
        if !is_paused(&env) {
            return Err(Error::NotPaused);
        }
        bump_instance(&env);
        set_paused(&env, false);
        events::unpaused(&env, &caller, env.ledger().timestamp());
        Ok(())
    }

    /// Permissionless keep-alive: extend the vault instance TTL so a paused
    /// vault can be kept warm during long investigations without requiring
    /// the owner to re-open the contract or submit a `RestoreFootprint`.
    pub fn keep_alive(env: Env) -> Result<(), Error> {
        bump_instance(&env);
        Ok(())
    }

    /// Read-only: whether the vault is currently under an emergency pause.
    pub fn is_paused(env: Env) -> bool {
        is_paused(&env)
    }
}
