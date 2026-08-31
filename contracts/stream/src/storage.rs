use soroban_sdk::{contracttype, Address};

// Bit-flags packed into `StreamInfo::flags`. Kept `pub` so cross-crate
// regression tests (e.g. `tests/audit_round_2_regression.rs::pause_resume_*`)
// and the `info().is_paused()`/`is_cancelled()`/`is_clawback_enabled()` getters
// can use them, but marked `#[doc(hidden)]` to keep the rustdoc contract API
// surface clean. Off-chain callers should use the `is_*()` getters rather than
// reading the bit values directly.
#[doc(hidden)]
pub const FLAG_PAUSED: u32 = 1;
#[doc(hidden)]
pub const FLAG_CLAWBACK_ENABLED: u32 = 1 << 1;
#[doc(hidden)]
pub const FLAG_CANCELLED: u32 = 1 << 2;

/// Current storage layout version for this contract.
/// Bump this and add an explicit migration check whenever a future
/// upgrade changes the shape of persisted `StreamInfo`/`Config` data.
pub const CURRENT_STORAGE_VERSION: u32 = 1;

#[contracttype]
pub enum DataKey {
    Sender,
    Recipient,
    Token,
    RatePerSecond,
    StartTime,
    EndTime,
    Withdrawn,
    PausedAt,
    Flags,
    ClawbackEnabled,
    Cancelled,
    /// Single-key representation of all stream fields.
    /// Replaces the 11 individual keys above for new writes — loaded in one
    /// storage read instead of eleven.
    Config,
    /// Legacy standalone copy of the current event sequence value.
    ///
    /// New writes persist this as part of `StreamInfo`/`Config` so it survives
    /// consolidated-key migrations. Older streams may still have this key until
    /// the first `save()` migrates them to the single-key layout.
    EventSequence,
    /// Lock for re-entrancy protection and concurrency control.
    Guard,
    /// Storage layout version, written once at `initialize()`.
    /// Future contract upgrades must check this before assuming the
    /// persisted `StreamInfo`/`Config` layout is compatible.
    StorageVersion,
    /// Optional operator address delegated by the sender.
    ///
    /// When set, the operator can perform sender-level actions (pause,
    /// cancel, clawback, top_up, extend_duration) on behalf of the sender.
    /// Absent key means no operator has been delegated.
    Operator,
    /// Seconds a stream must remain paused before `force_cancel` becomes
    /// callable by the recipient. Set once at `initialize()` from the
    /// value `DripFactory::create_stream` read from `GovernorConfig` at
    /// deploy time (governance-configurable per deployment; see
    /// `DripGovernor::set_force_cancel_pause_threshold`). Stored on the
    /// stream itself — rather than read cross-contract on every
    /// `force_cancel` call — because ADR-001 keeps this contract's hot
    /// path free of cross-contract calls; a stream deployed directly
    /// (bypassing the factory) falls back to the historical 30-day default.
    ForceCancelPauseThresholdSecs,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamInfo {
    pub sender: Address,
    pub recipient: Address,
    pub token: Address,
    pub rate_per_second: i128,
    pub start_time: u64,
    pub end_time: u64,
    pub withdrawn: i128,
    pub paused_at: u64,
    pub flags: u32,
    pub event_sequence: u64,
}

impl StreamInfo {
    pub fn is_paused(&self) -> bool {
        (self.flags & FLAG_PAUSED) != 0
    }

    pub fn is_cancelled(&self) -> bool {
        (self.flags & FLAG_CANCELLED) != 0
    }

    pub fn is_clawback_enabled(&self) -> bool {
        (self.flags & FLAG_CLAWBACK_ENABLED) != 0
    }
}
