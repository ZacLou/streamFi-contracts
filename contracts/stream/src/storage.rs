use soroban_sdk::{contracttype, Address, Env};

// Bit-flags packed into `StreamInfo::flags`. Kept `pub` so cross-crate regression tests (e.g. `tests/audit_round_2_regression.rs::pause_resume_*`)
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

// Current storage layout version for this contract.
// Bump this and add an explicit migration check whenever a future
// upgrade changes the shape of persisted `StreamInfo`/`Config` data.
pub const CURRENT_STORAGE_VERSION: u32 = 1;

// Reentrancy guard states.
pub const GUARD_NOT_ENTERED: u32 = 0;
pub const GUARD_ENTERED: u32 = 1;

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
    /// Replaces the 11 individual keys above for new writes - loaded in one
    /// storage read instead of eleven.
    Config,
    /// Monotonic identifier attached to every contract event.
    ///
    /// Consumers compare this value with the last sequence they processed
    /// after reconnecting so missing ledger events cannot go unnoticed.
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
    pub operator: Option<Address>,
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

    /// Marks the stream as cancelled by setting the `FLAG_CANCELLED` bit.
    pub fn mark_cancelled(&mut self) {
        self.flags |= FLAG_CANCELLED;
    }
}

/// Reads the current reentrancy guard state.
pub fn read_guard(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::Guard)
        .unwrap_or(GUARD_NOT_ENTERED)
}

/// Sets the reentrancy guard state.
pub fn write_guard(env: &Env, state: u32) {
    env.storage().instance().set(&DataKey::Guard, &state);
}
