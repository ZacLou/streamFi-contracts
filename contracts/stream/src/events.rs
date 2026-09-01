use soroban_sdk::{panic_with_error, symbol_short, Address, Env};

use crate::{storage::DataKey, Error};

/// Allocate the next event sequence before publishing its payload.
///
/// The sequence is now part of the persisted `StreamInfo`/`Config` payload so it
/// survives the consolidated state migration. Legacy streams without `Config` are
/// still readable via the fallback path for as long as they remain on the old
/// storage layout.
///
/// Boundary check: `current` is validated to prevent arithmetic overflow
/// on the sequence counter (which would silently consume future events).
fn next_sequence(env: &Env) -> u64 {
    let storage = env.storage().instance();
    let current = if storage.has(&DataKey::Config) {
        crate::state::load(env).event_sequence
    } else {
        storage.get::<_, u64>(&DataKey::EventSequence).unwrap_or(0)
    };
    let Some(next) = current.checked_add(1) else {
        panic_with_error!(env, Error::ArithmeticOverflow);
    };

    if storage.has(&DataKey::Config) {
        let mut info = crate::state::load(env);
        info.event_sequence = next;
        crate::state::save(env, &info);
    } else {
        storage.set(&DataKey::EventSequence, &next);
    }
    next
}

/// Validate that an `i128` amount is non-negative. A negative value is always
/// invalid for a payout, withdrawal, or balance field and indicates a corrupted
/// or unexpected state. Panics with `Error::InvalidAmount` so the caller
/// cannot silently emit a malformed event.
fn assert_non_negative_amount(env: &Env, value: i128) {
    if value < 0 {
        panic_with_error!(env, Error::InvalidAmount);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn created(
    env: &Env,
    sender: &Address,
    recipient: &Address,
    token: &Address,
    rate_per_second: i128,
    start_time: u64,
    end_time: u64,
    storage_version: u32,
) {
    assert_non_negative_amount(env, rate_per_second);

    let sequence = next_sequence(env);
    env.events().publish(
        (symbol_short!("created"), sender.clone(), sequence),
        (
            recipient.clone(),
            token.clone(),
            rate_per_second,
            start_time,
            end_time,
            storage_version,
        ),
    );
}

pub fn withdrawn(
    env: &Env,
    recipient: &Address,
    amount: i128,
    total_withdrawn: i128,
    remaining: i128,
) {
    // Boundary checks before any state mutation or event emission.
    assert_non_negative_amount(env, amount);
    assert_non_negative_amount(env, total_withdrawn);
    assert_non_negative_amount(env, remaining);

    let sequence = next_sequence(env);
    env.events().publish(
        (symbol_short!("withdrawn"), recipient.clone(), sequence),
        (amount, total_withdrawn, remaining),
    );
}

pub fn cancelled(env: &Env, sender: &Address, refund_amount: i128, withdrawn_so_far: i128) {
    assert_non_negative_amount(env, refund_amount);
    assert_non_negative_amount(env, withdrawn_so_far);

    let sequence = next_sequence(env);
    env.events().publish(
        (symbol_short!("cancelled"), sender.clone(), sequence),
        (refund_amount, withdrawn_so_far),
    );
}

/// Recipient-initiated cancellation via `force_cancel`, after the sender left
/// the stream paused past the threshold. Distinct from `cancelled` (the
/// sender/operator-initiated path via `cancel`) so consumers of the event log
/// can tell the two apart without correlating who signed the transaction.
pub fn force_cancelled(env: &Env, sender: &Address, refund_amount: i128, withdrawn_so_far: i128) {
    assert_non_negative_amount(env, refund_amount);
    assert_non_negative_amount(env, withdrawn_so_far);

    let sequence = next_sequence(env);
    env.events().publish(
        (symbol_short!("force_cxl"), sender.clone(), sequence),
        (refund_amount, withdrawn_so_far),
    );
}

pub fn paused(env: &Env, sender: &Address, paused_at: u64, withdrawable: i128) {
    assert_non_negative_amount(env, withdrawable);

    let sequence = next_sequence(env);
    env.events().publish(
        (symbol_short!("paused"), sender.clone(), sequence),
        (paused_at, withdrawable),
    );
}

pub fn resumed(env: &Env, sender: &Address, resumed_at: u64) {
    let sequence = next_sequence(env);
    env.events().publish(
        (symbol_short!("resumed"), sender.clone(), sequence),
        resumed_at,
    );
}

pub fn topped_up(env: &Env, sender: &Address, amount: i128, new_balance: i128) {
    assert_non_negative_amount(env, amount);
    assert_non_negative_amount(env, new_balance);

    let sequence = next_sequence(env);
    env.events().publish(
        (symbol_short!("topped_up"), sender.clone(), sequence),
        (amount, new_balance),
    );
}

pub fn clawback(env: &Env, sender: &Address, amount: i128) {
    assert_non_negative_amount(env, amount);

    let sequence = next_sequence(env);
    env.events().publish(
        (symbol_short!("clawback"), sender.clone(), sequence),
        amount,
    );
}

pub fn recipient_transferred(env: &Env, old_recipient: &Address, new_recipient: &Address) {
    let sequence = next_sequence(env);
    env.events().publish(
        (symbol_short!("xfer_rec"), old_recipient.clone(), sequence),
        new_recipient.clone(),
    );
}

pub fn operator_set(env: &Env, sender: &Address, operator: &Address) {
    let sequence = next_sequence(env);
    env.events().publish(
        (symbol_short!("set_op"), sender.clone(), sequence),
        operator.clone(),
    );
}

pub fn operator_revoked(env: &Env, sender: &Address) {
    let sequence = next_sequence(env);
    env.events()
        .publish((symbol_short!("rm_op"), sender.clone(), sequence), ());
}
