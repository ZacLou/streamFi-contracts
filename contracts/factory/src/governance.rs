use drip_governor::DripGovernorClient;
pub use drip_governor::GovernorConfig;
use soroban_sdk::{Address, Env};

use crate::Error;

/// Fetches the live protocol config from `governor` via a cross-contract call.
///
/// Returns `Err(GovernorNotResponding)` when the cross-contract call fails
/// (governor archived / not initialised / host error) instead of letting the
/// host trap bubble up as an opaque error.
pub fn config(env: &Env, governor: &Address) -> Result<GovernorConfig, Error> {
    // Cross-contract call — flattens the nested Result from try_config()
    // (outer = host error, inner = governor contract error) so callers see
    // a meaningful `GovernorNotResponding` instead of an opaque host trap.
    let result = DripGovernorClient::new(env, governor)
        .try_config()
        .map_err(|_| Error::GovernorNotResponding)?;
    result.map_err(|_| Error::GovernorNotResponding)
}

/// Enforces the governor-controlled rate/duration bounds on a new stream.
///
/// `rate_per_sec` and, for fixed-duration streams, the declared length must
/// respect the protocol parameters DripGovernor holds.
///
/// # Design Note: Creation vs Extension Bounds
///
/// This duration bound is enforced at stream creation time to cap initial upfront
/// commitment and scheduling horizons. Post-creation lifetime extensions via
/// `DripStream::extend_duration` and `DripStream::top_up_and_extend` are
/// intentionally unbounded by this initial creation cap, allowing ongoing streams
/// (e.g. payroll, recurring subscriptions) to continue without redeploying per ADR-001.
///
/// # Errors
///
/// This function uses `checked_sub` to safely compute stream duration, ensuring
/// it is robust against any call site — including future code paths that may
/// not pre-validate `end_time > start_time`. Returns `Err(ArithmeticOverflow)`
/// if `end_time < start_time` (underflow).
pub fn enforce_bounds(
    config: &GovernorConfig,
    rate_per_sec: i128,
    start_time: u64,
    end_time: u64,
    now: u64,
) -> Result<(), Error> {
    if rate_per_sec > config.max_rate_per_second {
        return Err(Error::RateExceedsMax);
    }

    // Bound how far ahead a stream may be scheduled.
    //
    // Without this, `start_time = now + 100 years` with `end_time = 0` is
    // accepted: the duration checks below only run for fixed-duration streams,
    // so an open-ended stream escapes every bound. The deposit is transferred
    // immediately and, when clawback is disabled, cannot be recovered before
    // `start_time` — which is to say, never in practice.
    //
    // `max_duration_seconds` is reused as the ceiling rather than introducing a
    // separate governor parameter: a stream that cannot begin for longer than
    // the protocol's longest permitted stream is locking funds beyond anything
    // a legitimate stream would. Reusing it also keeps this fix inside the
    // factory instead of migrating the governor's stored config.
    let start_offset = start_time.saturating_sub(now);
    if start_offset > config.max_duration_seconds {
        return Err(Error::StartTimeTooFarInFuture);
    }

    if end_time > 0 {
        let duration = end_time
            .checked_sub(start_time)
            .ok_or(Error::ArithmeticOverflow)?;
        if duration < config.min_duration_seconds {
            return Err(Error::DurationTooShort);
        }
        if duration > config.max_duration_seconds {
            return Err(Error::DurationExceedsMax);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    const HOUR: u64 = 3_600;
    const DAY: u64 = 86_400;
    const NOW: u64 = 1_700_000_000;

    fn config(env: &Env) -> GovernorConfig {
        GovernorConfig {
            fee_bps: 30,
            fee_recipient: Address::generate(env),
            min_duration_seconds: HOUR,
            max_duration_seconds: 30 * DAY,
            max_rate_per_second: 1_000_000,
            factory_address: Address::generate(env),
            force_cancel_pause_threshold_seconds: 30 * DAY,
        }
    }

    // ── start_time upper bound (#342) ───────────────────────────────────────
    //
    // `create_stream` rejects a backdated start, but nothing bounded how far
    // ahead one could be. With `end_time = 0` the duration checks below never
    // run, so `start_time = now + 100 years` was accepted: the deposit moves
    // immediately and, with clawback disabled, cannot be recovered before the
    // stream starts.

    #[test]
    fn open_ended_stream_far_in_the_future_is_rejected() {
        let env = Env::default();
        let cfg = config(&env);

        // 100 years out, open-ended — the exact case from the issue.
        let start = NOW + 100 * 365 * DAY;

        assert_eq!(
            enforce_bounds(&cfg, 1, start, 0, NOW),
            Err(Error::StartTimeTooFarInFuture)
        );
    }

    #[test]
    fn fixed_duration_stream_far_in_the_future_is_also_rejected() {
        // The bound applies regardless of end_time; a fixed-duration stream
        // scheduled beyond the ceiling locks funds just as long.
        let env = Env::default();
        let cfg = config(&env);
        let start = NOW + 100 * 365 * DAY;

        assert_eq!(
            enforce_bounds(&cfg, 1, start, start + DAY, NOW),
            Err(Error::StartTimeTooFarInFuture)
        );
    }

    #[test]
    fn start_exactly_at_the_ceiling_is_allowed() {
        // The bound is inclusive: an offset equal to max_duration_seconds is
        // the largest legitimate schedule, not one past it.
        let env = Env::default();
        let cfg = config(&env);
        let start = NOW + cfg.max_duration_seconds;

        assert_eq!(enforce_bounds(&cfg, 1, start, 0, NOW), Ok(()));
    }

    #[test]
    fn start_one_second_past_the_ceiling_is_rejected() {
        let env = Env::default();
        let cfg = config(&env);
        let start = NOW + cfg.max_duration_seconds + 1;

        assert_eq!(
            enforce_bounds(&cfg, 1, start, 0, NOW),
            Err(Error::StartTimeTooFarInFuture)
        );
    }

    #[test]
    fn immediate_start_is_unaffected() {
        // The overwhelmingly common case must not regress.
        let env = Env::default();
        let cfg = config(&env);

        assert_eq!(enforce_bounds(&cfg, 1, NOW, 0, NOW), Ok(()));
        assert_eq!(enforce_bounds(&cfg, 1, NOW, NOW + DAY, NOW), Ok(()));
    }

    #[test]
    fn a_start_time_already_in_the_past_does_not_underflow() {
        // `create_stream` rejects backdated starts before reaching here, but
        // enforce_bounds is also called from the batch path and must not panic
        // on a past timestamp. saturating_sub yields a zero offset.
        let env = Env::default();
        let cfg = config(&env);

        assert_eq!(enforce_bounds(&cfg, 1, NOW - 5 * DAY, 0, NOW), Ok(()));
    }

    // ── Existing bounds still hold ──────────────────────────────────────────

    #[test]
    fn rate_above_the_maximum_is_still_rejected() {
        let env = Env::default();
        let cfg = config(&env);

        assert_eq!(
            enforce_bounds(&cfg, cfg.max_rate_per_second + 1, NOW, 0, NOW),
            Err(Error::RateExceedsMax)
        );
    }

    #[test]
    fn duration_bounds_still_apply_to_fixed_duration_streams() {
        let env = Env::default();
        let cfg = config(&env);

        assert_eq!(
            enforce_bounds(&cfg, 1, NOW, NOW + cfg.min_duration_seconds - 1, NOW),
            Err(Error::DurationTooShort)
        );
        assert_eq!(
            enforce_bounds(&cfg, 1, NOW, NOW + cfg.max_duration_seconds + 1, NOW),
            Err(Error::DurationExceedsMax)
        );
    }

    #[test]
    fn end_before_start_still_reports_overflow() {
        let env = Env::default();
        let cfg = config(&env);

        assert_eq!(
            enforce_bounds(&cfg, 1, NOW + DAY, NOW, NOW),
            Err(Error::ArithmeticOverflow)
        );
    }
}
