use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotAuthorized = 1,
    StreamNotFound = 2,
    StreamCancelled = 3,
    StreamNotStarted = 4,
    StreamEnded = 5,
    NothingToWithdraw = 6,
    InsufficientDeposit = 7,
    InvalidTimeRange = 8,
    AlreadyPaused = 9,
    NotPaused = 10,
    ClawbackDisabled = 11,
    ArithmeticOverflow = 12,
    PauseThresholdNotMet = 13,
    AlreadyInitialized = 14,
    InvalidAmount = 15,
    ReentrancyForbidden = 16,
    OperatorAlreadySet = 17,
    NotInitialized = 18,
    /// The recipient is invalid (e.g. the all-zero Stellar account address, or identical to `sender`).
    InvalidRecipient = 19,
    /// The stream's `start_time` is in the past at initialization.
    BackdatedStream = 20,
    /// The stream has accrued tokens but is not funded enough to cover the requested withdrawal.
    StreamUnderfunded = 21,
}
