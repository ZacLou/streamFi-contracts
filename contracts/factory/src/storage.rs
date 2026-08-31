use soroban_sdk::{contracttype, Address, Vec};

/// Identifies which on-chain stream operation to estimate fees for.
///
/// Each variant corresponds to a distinct transaction shape with different
/// resource requirements (CPU instructions, read/write entries, etc.).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StreamOperation {
    /// Deploy a new payment stream via `DripFactory::create_stream`.
    /// Highest cost: contract deployment + multiple storage writes + token
    /// transfer + governor cross-contract call.
    CreateStream,
    /// Cancel an existing stream via `DripStream::cancel`. Moderate cost:
    /// single cross-contract call + settlement + event emission.
    CancelStream,
    /// Withdraw accrued funds from a stream via `DripStream::withdraw`.
    /// Lowest cost: single cross-contract call + event emission.
    Withdraw,
    /// Pause an active stream via `DripStream::pause`.
    PauseStream,
    /// Resume a paused stream via `DripStream::resume`.
    ResumeStream,
}

/// Result of a Soroban fee simulation for a stream operation.
///
/// Returned by `DripFactory::estimate_fee` so the UI can display the
/// estimated network cost to the user before they sign the transaction.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeEstimate {
    /// Estimated Soroban resource cost in stroops (1 XLM = 10_000_000 stroops).
    /// Calculated by the RPC simulation from actual CPU/RAM usage and the
    /// current network base fee.
    pub fee_stroops: i128,
    /// Estimated fee in human-readable XLM (fee_stroops / 10_000_000).
    pub fee_xlm: i128,
    /// Number of Soroban compute units (instructions) consumed by the
    /// simulated operation. Derived from the simulation result's
    /// `resources.cpu_instructions`.
    pub cpu_instructions: u32,
    /// Number of Soroban read/write ledger entries consumed.
    /// Derived from the simulation result's `resources.read_entries` and
    /// `resources.write_entries`.
    pub ledger_entries: u32,
}

/// A single request within a `create_batch_streams` call.
///
/// Each field mirrors the corresponding parameter of
/// [`DripFactory::create_stream`]; see that function's documentation
/// for descriptions of each field.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchStreamRequest {
    pub recipient: Address,
    pub token: Address,
    pub deposit: i128,
    pub rate_per_sec: i128,
    pub start_time: u64,
    pub end_time: u64,
}

/// Combined status of the DripFactory contract.
///
/// Combines `is_paused` and `protocol_fee_bps` into a single view struct so
/// UIs and indexers can query overall protocol health in a single RPC call.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FactoryStatus {
    pub is_paused: bool,
    /// Current protocol fee, or `None` when it could not be read.
    ///
    /// `None` means the governor is unreachable or the factory is not
    /// initialised — deliberately distinct from `Some(30)`, which means the
    /// governor is genuinely configured at 30 bps. Previously both collapsed
    /// to a bare `30`, so a caller could not tell a real fee from a fallback.
    ///
    /// Optional rather than making the whole call fallible so that a governor
    /// outage does not also hide `is_paused`, which this view exists to report.
    pub protocol_fee_bps: Option<u32>,
}

/// A page of stream IDs returned by `streams_by_sender` / `streams_by_recipient`,
/// paired with the total count so a caller can tell truncation from an
/// exhausted history without a separate `stream_count_by_*` round-trip.
///
/// `ids.len()` is capped at [`crate::query::MAX_PAGE_SIZE`] (100) regardless of
/// the requested `limit`. Compare `offset + ids.len()` against `total`: if it
/// is less, more pages remain (whether because `limit` was clamped or the
/// caller simply asked for a partial window).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamPage {
    pub ids: Vec<u64>,
    /// Total number of streams for this sender/recipient, independent of
    /// `offset`/`limit`. Equivalent to `stream_count_by_sender` /
    /// `stream_count_by_recipient`.
    pub total: u32,
}

/// Storage keys for the DripFactory contract.
///
/// The `#[contracttype]` macro serializes each variant as an XDR tagged union:
/// the discriminant (variant index) followed by the encoded inner type. This
/// means `DataKey::StreamAddr(42)` and `DataKey::StreamAddr(43)` are distinct
/// keys in Soroban's storage trie, each serialized as:
///   [discriminant: u32][stream_id: u64]
///
/// Similarly, `DataKey::BySender(address)` serializes as:
///   [discriminant: u32][address: XDR-encoded Address]
///
/// Storage is split across two tiers:
/// - **Instance storage**: Small, contract-scoped data that scales with the
///   number of operations (e.g., counters, config). Bounded by instance size limits.
/// - **Persistent storage**: Per-entity data that grows without bound (e.g.,
///   per-stream addresses, per-user indices). Avoids hitting instance size limits
///   as the protocol scales. Each entry has its own TTL and can be extended independently.

/// Current storage layout version for this contract.
///
/// Written once at `initialize()` and checked by `upgrade()` before
/// swapping the factory's own WASM, mirroring `DripStream::CURRENT_STORAGE_VERSION`.
/// Bump this and add an explicit migration path whenever a future upgrade
/// changes the shape of persisted state (`StreamCount`, `StreamAddr`, the
/// paged `BySender*`/`ByRecipient*` indices, `LastBumpedId`, etc.).
pub const CURRENT_STORAGE_VERSION: u32 = 1;

#[contracttype]
pub enum DataKey {
    /// **Instance storage.** Monotonically incrementing stream counter.
    /// Key: `DataKey::StreamCount` (no inner type, discriminant only)
    /// Value: `u64` — the next stream ID to assign
    StreamCount,

    /// **Persistent storage.** Maps stream ID to its deployed contract address.
    /// Key: `DataKey::StreamAddr(u64)` — the stream's unique ID
    /// Value: `Address` — the on-chain address of the deployed DripStream contract
    /// Serialization: XDR tagged union [discriminant: u32][stream_id: u64] → [contract_address: XDR Address]
    /// TTL: Extended to `ttl::EXTEND_TO` (200_000 ledgers) on creation
    StreamAddr(u64),

    /// **Persistent storage.** Index of all streams created by a given sender.
    /// Key: `DataKey::BySender(Address)` — the sender's Stellar address
    /// Value: `Vec<u64>` — list of stream IDs created by this sender, in creation order
    /// Serialization: XDR tagged union [discriminant: u32][sender: XDR Address] → [stream_ids: XDR Vec<u64>]
    /// TTL: Extended to `ttl::EXTEND_TO` (200_000 ledgers) on each new stream
    /// Note: Legacy pre-paged index; kept for backward-compatible reads and migration
    BySender(Address),

    /// **Persistent storage.** Index of all streams received by a given recipient.
    /// Key: `DataKey::ByRecipient(Address)` — the recipient's Stellar address
    /// Value: `Vec<u64>` — list of stream IDs where this address is the recipient, in creation order
    /// Serialization: XDR tagged union [discriminant: u32][recipient: XDR Address] → [stream_ids: XDR Vec<u64>]
    /// TTL: Extended to `ttl::EXTEND_TO` (200_000 ledgers) on each new stream
    /// Note: Legacy pre-paged index; kept for backward-compatible reads and migration
    ByRecipient(Address),

    /// **Instance storage.** WASM hash of the DripStream contract (for deployment).
    /// Key: `DataKey::StreamWasmHash` (no inner type, discriminant only)
    /// Value: `BytesN<32>` — SHA-256 hash of the stream contract WASM
    StreamWasmHash,

    /// **Instance storage.** Address of the DripGovernor contract.
    /// Key: `DataKey::GovernorAddress` (no inner type, discriminant only)
    /// Value: `Address` — the on-chain address of the DripGovernor contract
    GovernorAddress,

    /// **Instance storage.** Emergency-pause flag.
    /// Key: `DataKey::Paused` (no inner type, discriminant only)
    /// Value: `bool` — `true` while the protocol is under an emergency halt.
    /// A missing entry (e.g. a factory initialized before this feature
    /// existed) is treated as `false`/unpaused.
    Paused,

    /// **Instance storage.** Cursor for the bounded persistent-entry TTL
    /// walker. Each call into `pause`/`unpause`/`upgrade_stream_wasm`
    /// advances this cursor by `ttl::BATCH_LIMIT` IDs (modulo `StreamCount`)
    /// and bumps the persistent `StreamAddr(id)` TTLs in that window. This
    /// keeps the registry alive during idle periods independently of
    /// `create_stream` (which only bumps entries it touches).
    /// Key: `DataKey::LastBumpedId` (no inner type, discriminant only)
    /// Value: `u64` — the last stream ID whose persistent `StreamAddr` TTL
    /// was bumped by the walker. Missing entry is treated as `0`.
    LastBumpedId,

    /// **Instance storage.** Reentrancy guard for `create_stream`.
    /// Key: `DataKey::CreateLock` (no inner type, discriminant only)
    /// Value: `bool` — `true` while a `create_stream` call is mid-flight.
    /// `token` is caller-supplied and may be an untrusted/non-conforming
    /// contract; without this guard, a malicious `transfer` implementation
    /// could call back into `create_stream` before the outer call finishes
    /// and observe/mutate `StreamCount` and the registry indices twice for
    /// what should be a single atomic creation. A missing entry is treated
    /// as `false`/unlocked.
    CreateLock,

    /// **Persistent storage.** Paged sender index for bounded reads.
    /// Key: `DataKey::BySenderPage(Address, u32)` — sender plus zero-based page number
    /// Value: `Vec<u64>` — up to `query::MAX_PAGE_SIZE` stream IDs in creation order
    BySenderPage(Address, u32),

    /// **Persistent storage.** Paged recipient index for bounded reads.
    /// Key: `DataKey::ByRecipientPage(Address, u32)` — recipient plus zero-based page number
    /// Value: `Vec<u64>` — up to `query::MAX_PAGE_SIZE` stream IDs in creation order
    ByRecipientPage(Address, u32),

    /// **Persistent storage.** Number of legacy sender entries already copied
    /// into paged storage during an incremental migration.
    BySenderMigrationCursor(Address),

    /// **Persistent storage.** Original sender legacy-index length captured
    /// when incremental migration starts.
    BySenderLegacyCount(Address),

    /// **Persistent storage.** Number of legacy recipient entries already copied
    /// into paged storage during an incremental migration.
    ByRecipientMigrationCursor(Address),

    /// **Persistent storage.** Original recipient legacy-index length captured
    /// when incremental migration starts.
    ByRecipientLegacyCount(Address),

    /// **Persistent storage.** Total number of sender-indexed streams.
    /// Key: `DataKey::BySenderCount(Address)` — sender address
    /// Value: `u32` — total count used to derive page boundaries
    BySenderCount(Address),

    /// **Persistent storage.** Total number of recipient-indexed streams.
    /// Key: `DataKey::ByRecipientCount(Address)` — recipient address
    /// Value: `u32` — total count used to derive page boundaries
    ByRecipientCount(Address),

    /// **Instance storage.** Storage layout version this instance was
    /// initialized with. Written once at `initialize()`; checked by
    /// `upgrade()` before swapping WASM so an incompatible storage-layout
    /// change cannot be deployed onto existing state silently.
    /// Key: `DataKey::FactoryStorageVersion` (no inner type, discriminant only)
    /// Value: `u32`
    FactoryStorageVersion,
}
