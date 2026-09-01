use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    InvalidAmount = 1,
    ArithmeticOverflow = 2,
    LimitExceeded = 3,
    NotAuthorized = 4,
    /// `deposit`, `withdraw`, or `set_limit` was called while the vault is
    /// under an emergency pause.
    ContractPaused = 5,
    /// `pause` was called while the vault was already paused.
    AlreadyPaused = 6,
    /// `unpause` was called while the vault was not paused.
    NotPaused = 7,
    /// The vault has not been initialized yet.
    NotInitialized = 8,
    /// `initialize` was called on a vault that is already initialized.
    AlreadyInitialized = 9,
    /// A provided argument was invalid (e.g. proposing a zero-address owner).
    InvalidParam = 10,
    /// `accept_owner` was called but there is no pending owner to accept.
    NoPendingOwner = 11,
    /// `accept_owner` was called by an address that is not the pending owner.
    NotPendingOwner = 12,
    /// The token transfer did not move exactly the expected amount for `deposit`.
    DepositTransferFailed = 13,
    /// The token transfer did not move exactly the expected amount for `withdraw`.
    WithdrawTransferFailed = 14,
}
