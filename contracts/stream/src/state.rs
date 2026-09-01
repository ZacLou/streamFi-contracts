use soroban_sdk::{panic_with_error, Env};

use crate::storage::{DataKey, StreamInfo, FLAG_CANCELLED, FLAG_CLAWBACK_ENABLED};
use crate::Error;

/// Load the full stream state in a single storage read.
///
/// Tries the consolidated `Config` key first (written by all new
/// `initialize()` calls). Falls back to reading each field individually
/// for streams that were initialized before this optimisation landed —
/// this keeps older on-chain instances readable without a migration.
pub fn try_load(env: &Env) -> Result<StreamInfo, Error> {
    let s = env.storage().instance();

    // Fast path: stream was initialized with the consolidated key.
    if s.has(&DataKey::Config) {
        if let Some(info) = s.get::<_, StreamInfo>(&DataKey::Config) {
            return Ok(info);
        }
    }

    // Legacy path: read each field individually (pre-optimisation streams).
    // Reconstructs the packed `flags` bitfield from the old dedicated
    // ClawbackEnabled/Cancelled keys plus the existing Flags key (which
    // already held the Paused bit before the single-key consolidation).
    let mut flags: u32 = s.get(&DataKey::Flags).unwrap_or(0);
    if s.get(&DataKey::ClawbackEnabled).unwrap_or(false) {
        flags |= FLAG_CLAWBACK_ENABLED;
    }
    if s.get(&DataKey::Cancelled).unwrap_or(false) {
        flags |= FLAG_CANCELLED;
    }

    Ok(StreamInfo {
        sender: s.get(&DataKey::Sender).ok_or(Error::NotInitialized)?,
        recipient: s.get(&DataKey::Recipient).ok_or(Error::NotInitialized)?,
        token: s.get(&DataKey::Token).ok_or(Error::NotInitialized)?,
        rate_per_second: s
            .get(&DataKey::RatePerSecond)
            .ok_or(Error::NotInitialized)?,
        start_time: s.get(&DataKey::StartTime).ok_or(Error::NotInitialized)?,
        end_time: s.get(&DataKey::EndTime).ok_or(Error::NotInitialized)?,
        withdrawn: s.get(&DataKey::Withdrawn).unwrap_or(0),
        paused_at: s.get(&DataKey::PausedAt).unwrap_or(0),
        flags,
        event_sequence: s.get(&DataKey::EventSequence).unwrap_or(0),
    })
}

pub fn load(env: &Env) -> StreamInfo {
    match try_load(env) {
        Ok(info) => info,
        Err(err) => panic_with_error!(env, err),
    }
}

/// Pre-consolidation per-field storage keys.
///
/// Before the single-key `Config` representation, each `StreamInfo` field was
/// stored under its own key. `save()` used to mirror them alongside `Config`,
/// but `load()` reads only `Config` on its fast path, so those mirrors were
/// write-only dead weight — ~10 `instance().set()` calls on every mutation
/// (`withdraw`/`pause`/`resume`/`cancel`/`top_up`/`extend_duration`) for data
/// no code path reads. They are removed once, on the first `save()` of a
/// pre-consolidation stream, after which `save()` writes only `Config`.
const LEGACY_STATE_KEYS: [DataKey; 12] = [
    DataKey::Sender,
    DataKey::Recipient,
    DataKey::Token,
    DataKey::RatePerSecond,
    DataKey::StartTime,
    DataKey::EndTime,
    DataKey::Withdrawn,
    DataKey::PausedAt,
    DataKey::Flags,
    DataKey::ClawbackEnabled,
    DataKey::Cancelled,
    DataKey::EventSequence,
];

/// Persist the entire stream state in a single storage write.
///
/// Writes only the consolidated `Config` key. `load()` reads `Config` on its
/// fast path, so the legacy per-field keys are never read once `Config` exists
/// (and every stream `initialize()`d since the consolidation has `Config`).
///
/// One-time legacy migration: the first `save()` on a pre-consolidation stream
/// (per-field keys present, no `Config`) supersedes them, so we remove them —
/// reclaiming the entries/rent they occupy — then write `Config`. Subsequent
/// saves see `Config` present and skip straight to the single write.
pub fn save(env: &Env, info: &StreamInfo) {
    let s = env.storage().instance();
    if !s.has(&DataKey::Config) {
        for legacy in LEGACY_STATE_KEYS.iter() {
            if s.has(legacy) {
                s.remove(legacy);
            }
        }
    }
    s.set(&DataKey::Config, info);
}

pub fn assert_not_cancelled(info: &StreamInfo) -> Result<(), Error> {
    if info.is_cancelled() {
        Err(Error::StreamCancelled)
    } else {
        Ok(())
    }
}

/// Maximum allowed re-entrancy depth. Must never be > 1 in production —
/// Soroban's execution model makes re-entrancy structurally impossible,
/// so any depth > 0 signals a bug or an emerging cross-contract callback
/// capability. Kept as a named constant so a future protocol upgrade that
/// introduces hooks can be gated here rather than in every call site.
pub const MAX_REENTRANCY_DEPTH: u32 = 1;

/// Acquire the re-entrancy guard.
///
/// Uses a depth counter stored at [`DataKey::Guard`] instead of a boolean
/// flag. The counter approach:
///
/// - Detects unbalanced acquire / release (depth would underflow).
/// - Allows controlled re-entrancy up to [`MAX_REENTRANCY_DEPTH`] if the
///   protocol ever supports it.
/// - Prevents the "two simultaneous acquires both see 0" TOCTOU that a
///   plain boolean check-then-set is theoretically vulnerable to.
///
/// In Soroban's synchronous, single-shot execution model the TOCTOU is
/// not exploitable today; the counter is defense-in-depth for future
/// protocol capabilities (hooks, cross-contract callbacks, etc.).
pub fn lock(env: &Env) -> Result<(), Error> {
    let s = env.storage().instance();
    let depth: u32 = s.get(&DataKey::Guard).unwrap_or(0);
    if depth >= MAX_REENTRANCY_DEPTH {
        return Err(Error::ReentrancyForbidden);
    }
    s.set(&DataKey::Guard, &(depth + 1));
    Ok(())
}

/// Release the re-entrancy guard.
///
/// Decrements the depth counter. If the counter is already zero (i.e.
/// `unlock` was called without a matching `lock`), the call is silently
/// swallowed — the state is already unlocked and panicking here would
/// unnecessarily block legitimate code paths.
pub fn unlock(env: &Env) {
    let s = env.storage().instance();
    let depth: u32 = s.get(&DataKey::Guard).unwrap_or(0);
    if depth > 0 {
        s.set(&DataKey::Guard, &(depth - 1));
    }
}

/// Execute `f` under the re-entrancy guard, releasing the lock afterwards.
///
/// This is the preferred entry point for all state-mutating contract
/// methods. It guarantees that the guard is released on every exit path —
/// early `return`, `?` propagation, or normal completion — without
/// requiring the caller to remember a manual `unlock()` call.
///
/// # Examples
///
/// ```ignore
/// pub fn withdraw(env: Env, amount: i128) -> Result<i128, Error> {
///     state::with_guard(&env, |env| Self::_withdraw(env, amount))
/// }
/// ```
pub fn with_guard<R>(env: &Env, f: impl FnOnce(&Env) -> Result<R, Error>) -> Result<R, Error> {
    lock(env)?;
    let result = f(env);
    unlock(env);
    result
}
