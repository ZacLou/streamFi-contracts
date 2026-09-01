use soroban_sdk::Env;

use drip_common::{TTL_EXTEND_TO, TTL_THRESHOLD};

/// Maximum safe duration a stream may remain paused before the instance
/// storage TTL window is no longer sufficient to resume it safely.
///
/// A single `extend_ttl` bump only renews the instance record to
/// `TTL_EXTEND_TO` (200_000 ledgers). Any pause that exceeds the window can
/// leave the stream archived before a normal `resume()` or `force_cancel()`
/// call can run.
pub const MAX_PAUSE_SECS: u64 = 2_592_000; // 30 days

pub fn bump(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
}
