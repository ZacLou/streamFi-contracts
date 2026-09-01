#![cfg(test)]

//! Property-based tests for `math::streamed_amount` and `math::withdrawable`
//! boundary behaviour (issue #444).
//!
//! These tests target the pure math functions directly rather than the
//! contract entry points. `streamed_amount` only inspects `is_paused()`,
//! not `is_cancelled()` — cancellation is enforced at the contract layer
//! (`DripStream::streamed_total` / `withdrawable` short-circuit to 0 when
//! `is_cancelled()` is true). The post-cancellation property below documents
//! that the math function itself is cancellation-agnostic so future
//! refactors don't silently change the layering.

extern crate std;

use proptest::prelude::*;

use soroban_sdk::{testutils::Address as _, testutils::Ledger, Address, Env};

use crate::math::{streamed_amount, withdrawable};
use crate::storage::{StreamInfo, FLAG_CANCELLED, FLAG_PAUSED};

/// Build a `StreamInfo` with the given timing fields and sensible defaults
/// for the rest. `flags` lets the caller set the paused / cancelled bits.
fn make_info(
    env: &Env,
    rate_per_second: i128,
    start_time: u64,
    end_time: u64,
    withdrawn: i128,
    paused_at: u64,
    flags: u32,
) -> StreamInfo {
    StreamInfo {
        sender: Address::generate(env),
        recipient: Address::generate(env),
        token: Address::generate(env),
        rate_per_second,
        start_time,
        end_time,
        withdrawn,
        paused_at,
        flags,
        event_sequence: 0,
    }
}

proptest! {
    /// When `end_time == start_time` and `now >= end_time`, the elapsed time
    /// is clamped to `end_time - start_time == 0`, so `streamed_amount` must
    /// be 0 regardless of the rate.
    #[test]
    fn streamed_zero_when_end_equals_start(
        rate in 1i128..1_000_000_000,
        start in 1_000u64..1_000_000_000,
        elapsed in 0u64..1_000_000,
    ) {
        let env = Env::default();
        let info = make_info(
            &env,
            rate,
            start,
            start, // end == start
            0,
            0,
            0,
        );
        env.ledger().set_timestamp(start + elapsed);
        let result = streamed_amount(&env, &info).unwrap();
        prop_assert_eq!(result, 0);
    }

    /// When `now < start_time`, nothing has streamed yet — even if `end_time`
    /// is 0 (open-ended) or huge.
    #[test]
    fn streamed_zero_before_start(
        rate in 1i128..1_000_000_000,
        start in 5_000u64..1_000_000_000,
        end_offset in 0u64..1_000_000, // 0 => open-ended
        deficit in 1u64..4_000,        // now is `deficit` seconds before start
    ) {
        let env = Env::default();
        let end_time = if end_offset == 0 { 0 } else { start + end_offset };
        let info = make_info(&env, rate, start, end_time, 0, 0, 0);
        env.ledger().set_timestamp(start - deficit);
        let result = streamed_amount(&env, &info).unwrap();
        prop_assert_eq!(result, 0);
    }

    /// Open-ended stream (`end_time == 0`): once `now >= start_time`,
    /// `streamed_amount == rate * (now - start_time)` with no clamping.
    #[test]
    fn streamed_open_ended_is_rate_times_elapsed(
        rate in 1i128..1_000_000,
        start in 1_000u64..1_000_000,
        elapsed in 1u64..1_000_000,
    ) {
        let env = Env::default();
        let info = make_info(&env, rate, start, 0, 0, 0, 0);
        let now = start + elapsed;
        env.ledger().set_timestamp(now);
        let result = streamed_amount(&env, &info).unwrap();
        let expected = rate.checked_mul(elapsed as i128).unwrap();
        prop_assert_eq!(result, expected);
    }

    /// Bounded stream (`end_time > 0`): once `now >= end_time`, accrual
    /// is clamped to `rate * (end_time - start_time)`.
    #[test]
    fn streamed_bounded_clamps_at_end_time(
        rate in 1i128..1_000_000,
        start in 1_000u64..1_000_000,
        duration in 1u64..1_000_000,
        extra in 0u64..1_000_000, // how far past end_time we read
    ) {
        let env = Env::default();
        let end_time = start + duration;
        let info = make_info(&env, rate, start, end_time, 0, 0, 0);
        env.ledger().set_timestamp(end_time + extra);
        let result = streamed_amount(&env, &info).unwrap();
        let expected = rate.checked_mul(duration as i128).unwrap();
        prop_assert_eq!(result, expected);
    }

    /// Exactly at `end_time` the stream should equal the full contracted
    /// amount (`rate * duration`) — the `now > end_time` clamp is strict,
    /// so `now == end_time` still uses `now`.
    #[test]
    fn streamed_at_end_time_is_full_amount(
        rate in 1i128..1_000_000,
        start in 1_000u64..1_000_000,
        duration in 1u64..1_000_000,
    ) {
        let env = Env::default();
        let end_time = start + duration;
        let info = make_info(&env, rate, start, end_time, 0, 0, 0);
        env.ledger().set_timestamp(end_time);
        let result = streamed_amount(&env, &info).unwrap();
        // at exactly end_time, elapsed == duration
        prop_assert_eq!(result, rate.checked_mul(duration as i128).unwrap());
    }

    /// Overflow: when `rate * elapsed` would overflow `i128`, the function
    /// must return `Err(ArithmeticOverflow)` rather than panicking or wrapping.
    #[test]
    fn streamed_overflow_returns_error(
        base in 1i128..(i128::MAX / 2),
    ) {
        let env = Env::default();
        let start: u64 = 1;
        let elapsed = (i128::MAX as u128 / base as u128 + 1) as u64;
        let now = start.checked_add(elapsed);
        // open-ended so no end_time clamp interferes
        let info = make_info(&env, base, start, 0, 0, 0, 0);
        if let Some(now) = now {
            env.ledger().set_timestamp(now);
            let result = streamed_amount(&env, &info);
            prop_assert!(
                matches!(result, Err(crate::errors::Error::ArithmeticOverflow)),
                "expected ArithmeticOverflow, got {:?}",
                result
            );
        }
    }

    /// Monotonicity: for a non-paused, non-cancelled, bounded or open-ended
    /// stream, `streamed_amount` is monotonically non-decreasing as `now`
    /// increases.
    #[test]
    fn streamed_monotonic_in_now(
        rate in 1i128..1_000_000,
        start in 1_000u64..1_000_000,
        end_offset in 0u64..1_000_000, // 0 => open-ended
        t1 in 0u64..2_000_000,
        t2_delta in 0u64..1_000_000,
    ) {
        let env = Env::default();
        let end_time = if end_offset == 0 { 0 } else { start + end_offset };
        let info = make_info(&env, rate, start, end_time, 0, 0, 0);

        let now1 = t1;
        env.ledger().set_timestamp(now1);
        let r1 = streamed_amount(&env, &info).unwrap();

        let now2 = now1 + t2_delta;
        env.ledger().set_timestamp(now2);
        let r2 = streamed_amount(&env, &info).unwrap();

        prop_assert!(
            r2 >= r1,
            "streamed decreased as now increased: r1={} r2={} now1={} now2={}",
            r1, r2, now1, now2
        );
    }

    /// Pause freezes accrual: a paused stream's `streamed_amount` does not
    /// grow as `now` increases past `paused_at`, as long as the reads stay
    /// within the stream's bounded lifetime (or the stream is open-ended).
    /// The frozen value equals `rate * (paused_at - start_time)`.
    ///
    /// NOTE: production `math::streamed_amount` clamps `now > end_time` to
    /// `end_time` *before* the pause branch, so once `now` passes `end_time`
    /// the value jumps to `rate * (end_time - start)` regardless of pause.
    /// That case is covered separately by
    /// `pause_clamps_to_end_time_when_paused_beyond_end`. To keep this
    /// property focused on the pause freeze we build `end_time` so the
    /// second read stays at or before it.
    #[test]
    fn pause_freezes_accrual(
        rate in 1i128..1_000_000,
        start in 1_000u64..1_000_000,
        pause_elapsed in 1u64..500_000,    // paused_at = start + pause_elapsed
        end_kind in 0u8..2,                // 0 => open-ended, 1 => short, 2 => long
        further in 0u64..500_000,          // how much later we re-read
    ) {
        let env = Env::default();
        let paused_at = start + pause_elapsed;
        // open-ended, or bounded well past `paused_at + further` so the
        // end_time clamp never triggers for either read.
        let end_time = match end_kind {
            0 => 0u64,
            1 => paused_at + 2 * further + 1,
            _ => paused_at + 10 * further + 1_000,
        };
        // Guard against any accidental overflow when building end_time.
        prop_assume!(end_time == 0 || end_time > paused_at + further);

        let info = make_info(&env, rate, start, end_time, 0, paused_at, FLAG_PAUSED);
        let frozen_expected = rate.checked_mul(pause_elapsed as i128).unwrap();

        // Read at paused_at
        env.ledger().set_timestamp(paused_at);
        let r1 = streamed_amount(&env, &info).unwrap();
        prop_assert_eq!(r1, frozen_expected);

        // Read further along — should stay frozen
        env.ledger().set_timestamp(paused_at + further);
        let r2 = streamed_amount(&env, &info).unwrap();
        prop_assert_eq!(r2, frozen_expected);
    }

    /// Pause-then-clamp: when a stream is paused AND the read time is past
    /// `end_time`, `effective_now` is clamped to `end_time`, so
    /// `streamed_amount == rate * (end_time - start_time)` — no more.
    #[test]
    fn pause_clamps_to_end_time_when_paused_beyond_end(
        rate in 1i128..1_000_000,
        start in 1_000u64..1_000_000,
        duration in 1u64..1_000_000,
        pause_after_end in 1u64..1_000_000, // paused_at = end_time + pause_after_end
    ) {
        let env = Env::default();
        let end_time = start + duration;
        let paused_at = end_time + pause_after_end;
        let info = make_info(&env, rate, start, end_time, 0, paused_at, FLAG_PAUSED);

        env.ledger().set_timestamp(paused_at + 1_000);
        let result = streamed_amount(&env, &info).unwrap();
        // effective_now clamps to end_time, elapsed = end_time - start
        prop_assert_eq!(result, rate.checked_mul(duration as i128).unwrap());
    }

    /// Post-cancellation: the `math::streamed_amount` function does NOT consult
    /// the cancelled flag — cancellation is enforced at the contract layer
    /// (`DripStream::streamed_total` returns 0 when `is_cancelled()`). So
    /// calling the math function directly on a cancelled-but-not-paused stream
    /// still returns `rate * elapsed`. This test documents that boundary so
    /// future refactors don't silently change the layering.
    #[test]
    fn math_ignores_cancelled_flag(
        rate in 1i128..1_000_000,
        start in 1_000u64..1_000_000,
        elapsed in 1u64..1_000_000,
    ) {
        let env = Env::default();
        // cancelled but not paused
        let info = make_info(&env, rate, start, 0, 0, 0, FLAG_CANCELLED);
        env.ledger().set_timestamp(start + elapsed);
        let result = streamed_amount(&env, &info).unwrap();
        prop_assert_eq!(result, rate.checked_mul(elapsed as i128).unwrap());
    }

    /// `withdrawable` never exceeds `streamed`, and is `max(0, streamed - withdrawn)`.
    #[test]
    fn withdrawable_never_exceeds_streamed(
        rate in 1i128..1_000_000,
        start in 1_000u64..1_000_000,
        elapsed in 0u64..1_000_000,
        withdrawn in 0i128..1_000_000_000,
    ) {
        let env = Env::default();
        let info = make_info(&env, rate, start, 0, withdrawn, 0, 0);
        env.ledger().set_timestamp(start + elapsed);
        let streamed = streamed_amount(&env, &info).unwrap();
        let w = withdrawable(&env, &info).unwrap();
        prop_assert!(w <= streamed, "withdrawable {} > streamed {}", w, streamed);
        let expected = streamed.saturating_sub(withdrawn).max(0);
        prop_assert_eq!(w, expected);
    }

    /// `withdrawable` never returns a negative value, no matter how large
    /// `withdrawn` is relative to `streamed`.
    #[test]
    fn withdrawable_never_negative(
        rate in 1i128..1_000_000,
        start in 1_000u64..1_000_000,
        elapsed in 0u64..1_000_000,
        withdrawn in 0i128..2_000_000_000,
    ) {
        let env = Env::default();
        let info = make_info(&env, rate, start, 0, withdrawn, 0, 0);
        env.ledger().set_timestamp(start + elapsed);
        let w = withdrawable(&env, &info).unwrap();
        prop_assert!(w >= 0, "withdrawable negative: {}", w);
    }

    /// `withdrawable` on a paused open-ended stream stays flat as `now`
    /// increases, mirroring `streamed_amount`'s freeze.
    #[test]
    fn withdrawable_freezes_when_paused(
        rate in 1i128..1_000_000,
        start in 1_000u64..1_000_000,
        pause_elapsed in 1u64..1_000_000,
        further in 0u64..1_000_000,
    ) {
        let env = Env::default();
        let paused_at = start + pause_elapsed;
        // open-ended so no end_time clamp interferes
        let info = make_info(&env, rate, start, 0, 0, paused_at, FLAG_PAUSED);
        let expected = rate.checked_mul(pause_elapsed as i128).unwrap();

        env.ledger().set_timestamp(paused_at);
        let w1 = withdrawable(&env, &info).unwrap();
        prop_assert_eq!(w1, expected);

        env.ledger().set_timestamp(paused_at + further);
        let w2 = withdrawable(&env, &info).unwrap();
        prop_assert_eq!(w2, expected);
    }
}
