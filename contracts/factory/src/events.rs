use soroban_sdk::{symbol_short, Address, Env};

/// Emergency-pause observability events.
///
/// The factory's `pause`/`unpause` are already idempotent — a redundant call
/// reverts with `AlreadyPaused`/`NotPaused` rather than silently no-op'ing.
/// These events close the remaining gap: they let off-chain infrastructure
/// (indexers, relayers) positively confirm that a state transition committed,
/// rather than having to infer it from a bare "ok" that an ambiguous or
/// rate-limited RPC response may have dropped or duplicated. A relayer that
/// retried after a lost response can reconcile against the emitted event
/// instead of re-issuing a call it can no longer distinguish as a no-op.
///
/// Publication and the `set_paused` storage write are part of the same Soroban
/// transaction, so either both commit or both roll back — an event is never
/// emitted for a transition that did not actually persist.
///
/// Emitted when the factory transitions from unpaused to paused.
///
/// Topics: `("paused", governor)` — the governor that authorized the halt.
/// Data:   `paused_at` — the ledger timestamp at which the halt took effect.
pub fn paused(env: &Env, governor: &Address, paused_at: u64) {
    env.events()
        .publish((symbol_short!("paused"), governor.clone()), paused_at);
}

/// Emitted when the factory transitions from paused back to unpaused.
///
/// Topics: `("unpaused", governor)` — the governor that lifted the halt.
/// Data:   `resumed_at` — the ledger timestamp at which creation resumed.
pub fn unpaused(env: &Env, governor: &Address, resumed_at: u64) {
    env.events()
        .publish((symbol_short!("unpaused"), governor.clone()), resumed_at);
}

pub fn upgraded(env: &Env, governor: &Address, upgraded_at: u64) {
    env.events()
        .publish((symbol_short!("upgraded"), governor.clone()), upgraded_at);
}

/// Emitted on every successfully created stream, once the governed protocol
/// fee has been routed to its recipient.
///
/// Topics: `("fee", fee_recipient)` — the address that received the fee.
/// Data:   `fee` — the amount routed (in stroops). Always the deposit's
///         `fee_bps / 10_000`, and `0` when the deposit is small enough that
///         the basis-point rounding yields nothing (or `fee_bps` is 0).
pub fn protocol_fee_charged(env: &Env, fee_recipient: &Address, fee: i128) {
    env.events()
        .publish((symbol_short!("fee"), fee_recipient.clone()), fee);
}
