use soroban_sdk::{contracttype, Address};

/// Protocol administration roles.
///
/// Each role gates a distinct slice of governor state, so independent wallets
/// can own fee policy and rate/duration bounds without sharing a single
/// all-powerful key:
///
/// - `Admin`       — grant and revoke roles (including `Admin` itself).
/// - `FeeManager`  — `set_fee_bps`, `set_fee_recipient`.
/// - `RateManager` — `set_max_rate`, `set_min_duration`.
/// - `Pauser`      — `governor_pause`, `governor_unpause`, `pause_factory`, `unpause_factory`.
///
/// A role may be held by any number of accounts, and one account may hold any
/// combination of roles.
#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Admin,
    FeeManager,
    RateManager,
    Pauser,
}

/// Composite key identifying a single (role, account) grant.
///
/// Soroban `contracttype` enum variants carry at most one payload, so the
/// (role, account) pair is wrapped in this struct rather than expressed as a
/// two-field `DataKey` variant.
#[contracttype]
#[derive(Clone)]
pub struct RoleKey {
    pub role: Role,
    pub account: Address,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Fee in basis points (e.g. 30 = 0.3%)
    FeeBps,
    /// Address that receives protocol fees
    FeeRecipient,
    /// Minimum allowed stream duration in seconds
    MinDurationSeconds,
    /// Maximum rate per second in stroops
    MaxRatePerSecond,
    /// Maximum allowed stream duration in seconds
    MaxDurationSeconds,
    /// The DripFactory contract this governor controls
    FactoryAddress,
    /// Seconds a stream must remain paused before `DripStream::force_cancel`
    /// becomes callable by the recipient.
    ForceCancelPauseThresholdSeconds,
    /// Presence marks that `account` holds `role`. The stored value is an
    /// unused `bool`; membership is expressed entirely by the key existing.
    Role(RoleKey),
    /// Number of accounts currently holding `Role::Admin`. Tracked so the last
    /// admin can never be revoked, which would freeze governance permanently.
    AdminCount,
    /// Emergency-pause flag. When `true`, parameter writes are blocked.
    Paused,
    /// Index of all accounts currently holding a given role.
    /// Maintained alongside `grant`/`revoke` so role membership can be
    /// enumerated on-chain without replaying every event from genesis.
    RoleMembers(Role),
    /// Pending authority address for 2-step transfer.
    /// Set by `propose_authority`, consumed by `accept_authority`.
    PendingAuthority,
    /// The admin who proposed the authority transfer.
    /// Used to revoke their Admin role when the transfer is accepted.
    PendingAuthorityProposer,
}
