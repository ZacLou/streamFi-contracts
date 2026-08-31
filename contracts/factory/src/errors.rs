use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    InvalidDeposit = 2,
    InvalidRate = 3,
    InvalidTimeRange = 4,
    InsufficientDeposit = 5,
    BackdatedStream = 6,
    AlreadyInitialized = 7,
    RateExceedsMax = 8,
    DurationTooShort = 9,
    ArithmeticOverflow = 10,
    /// The factory is under an emergency pause; new stream creation is halted.
    ContractPaused = 11,
    /// `pause` was called while the factory was already paused.
    AlreadyPaused = 12,
    /// `unpause` was called while the factory was not paused.
    NotPaused = 13,
    DurationExceedsMax = 14,
    /// The governor contract did not respond (archived, not initialised,
    /// or a host-level error occurred during the cross-contract call).
    GovernorNotResponding = 15,
    /// `create_batch_streams` was called with an empty `requests` vector.
    EmptyBatch = 16,
    /// `create_batch_streams` requests exceeded `MAX_BATCH_SIZE`.
    BatchTooLarge = 17,
    /// The recipient is invalid (e.g. the all-zero Stellar account address, or identical to `sender`).
    InvalidRecipient = 22,
    /// Another `create_stream` call is already in progress (reentrancy guard).
    CreateLocked = 23,
    /// The deposit transfer from `sender` to the factory did not arrive.
    DepositTransferFailed = 24,
    /// The deposit forward from the factory to the deployed stream did not arrive.
    StreamFundingFailed = 25,
    /// The WASM hash provided to `upgrade_stream_wasm` is all zeros (invalid).
    InvalidWasmHash = 26,
    /// Stream duration is invalid (end_time <= start_time).
    InvalidDuration = 27,
    /// The token address is invalid (e.g. the all-zero Stellar account address).
    InvalidToken = 28,
    /// The stream start is too far in the future for the protocol's allowed scheduling window.
    /// `start_time` is further ahead than the protocol's maximum stream
    /// duration allows.
    ///
    /// An unbounded `start_time` locks the deposit for as long as the caller
    /// likes: with `end_time = 0` no duration bound applies, and if clawback
    /// is disabled there is no way to recover the funds before the stream
    /// starts.
    StartTimeTooFarInFuture = 29,
    /// `upgrade` was called with a WASM hash whose storage layout version
    /// does not match `DataKey::FactoryStorageVersion`. Prevents swapping in
    /// code that assumes an incompatible shape for persisted state
    /// (`StreamCount`, `StreamAddr`, the paged indices, etc.) without an
    /// explicit migration.
    StorageVersionMismatch = 30,
}
