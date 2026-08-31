#![no_std]

mod deploy;
mod errors;
mod events;
mod governance;
mod index;
mod pause;
mod query;
pub mod storage;
#[cfg(test)]
mod tests;
pub mod ttl;

// Import `token` as `tok` to avoid shadowing by any `token: Address` parameter.
use soroban_sdk::{
    contract, contractimpl, panic_with_error, token as tok, Address, BytesN, Env, IntoVal, Vec,
};

use drip_common::is_zero_address;

pub use errors::Error;
use storage::DataKey;
pub use storage::{BatchStreamRequest, FactoryStatus, FeeEstimate, StreamOperation, StreamPage};

/// Maximum number of streams accepted by a single `create_batch_streams`
/// (and `cancel_batch_streams`/`stream_addresses`) call. The batch
/// performs one governor cross-contract config call, and each stream
/// performs two `token::transfer`s, a contract deploy + `initialize`
/// invoke, and three persistent writes with TTL extensions (~2.5M CPU
/// instructions).
/// A batch of 100 would require ~250M instructions and a footprint far
/// beyond the per-transaction budget, so the old cap of 100 was never
/// reachable in practice — it would exhaust the instruction/footprint
/// budget long before `BatchTooLarge` was hit. Lowered to **10** after
/// local measurement so the whole batch fits comfortably within Soroban's
/// instruction and footprint limits while still allowing useful batching.
/// Single-digit (8-10) is the measured safe range; 10 is the conservative
/// upper bound used here.
pub const MAX_BATCH_SIZE: u32 = 10;

/// Returns true when `hash` is an all-zero 32-byte WASM hash.
fn is_zero_wasm_hash(env: &Env, hash: &BytesN<32>) -> bool {
    *hash == BytesN::from_array(env, &[0u8; 32])
}
#[contract]
pub struct DripFactory;

#[contractimpl]
impl DripFactory {
    /// One-time setup — called by the deploy script.
    ///
    /// Guards against re-initialization: without this check, anyone could
    /// call `initialize` again to point the factory at an attacker-controlled
    /// `stream_wasm_hash` or `governor`, hijacking every subsequent
    /// `create_stream` call.
    pub fn initialize(env: Env, stream_wasm_hash: BytesN<32>, governor: Address) {
        if env.storage().instance().has(&DataKey::StreamCount) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        ttl::bump_instance(&env);

        env.storage()
            .instance()
            .set(&DataKey::StreamWasmHash, &stream_wasm_hash);
        env.storage()
            .instance()
            .set(&DataKey::GovernorAddress, &governor);
        env.storage().instance().set(&DataKey::StreamCount, &0_u64);
        env.storage().instance().set(
            &DataKey::FactoryStorageVersion,
            &storage::CURRENT_STORAGE_VERSION,
        );
    }

    /// Deploy a new DripStream and register it.
    ///
    /// The caller (`sender`) must pass their address explicitly — Soroban has no
    /// implicit `msg.sender`. `sender.require_auth()` enforces that the transaction
    /// is signed by the address it claims to be.
    #[allow(clippy::too_many_arguments)]
    pub fn create_stream(
        env: Env,
        sender: Address, // the stream creator / funder
        recipient: Address,
        token: Address, // Stellar asset contract address
        deposit: i128,
        rate_per_sec: i128,
        start_time: u64,
        end_time: u64,
        clawback: bool,
    ) -> Result<u64, Error> {
        // ── Auth ─────────────────────────────────────────────────────────
        sender.require_auth();

        // ── Emergency pause ──────────────────────────────────────────────
        // Checked before any validation or state access so a halted protocol
        // rejects new streams immediately, without pulling a deposit or paying
        // a TTL extension. Already-deployed streams are independent contracts
        // and are unaffected by this flag.
        if pause::is_paused(&env) {
            return Err(Error::ContractPaused);
        }

        // ── Validation ───────────────────────────────────────────────────
        let now = Self::validate_stream_request(
            &env,
            &sender,
            &recipient,
            &token,
            deposit,
            rate_per_sec,
            start_time,
            end_time,
        )?;

        // ── Governor-controlled bounds ──────────────────────────────────────
        let governor: Address = env
            .storage()
            .instance()
            .get(&DataKey::GovernorAddress)
            .ok_or(Error::NotInitialized)?;
        let config = governance::config(&env, &governor)?;

        Self::create_stream_with_config(
            env,
            &config,
            now,
            sender,
            recipient,
            token,
            deposit,
            rate_per_sec,
            start_time,
            end_time,
            clawback,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_stream_request(
        env: &Env,
        sender: &Address,
        recipient: &Address,
        token: &Address,
        deposit: i128,
        rate_per_sec: i128,
        start_time: u64,
        end_time: u64,
    ) -> Result<u64, Error> {
        // ── Recipient validation ─────────────────────────────────────────
        if is_zero_address(env, recipient) || recipient == sender {
            return Err(Error::InvalidRecipient);
        }

        // ── Token validation ─────────────────────────────────────────────
        if is_zero_address(env, token) {
            return Err(Error::InvalidToken);
        }

        // ── Validation ───────────────────────────────────────────────────
        // Fail early: all input checks run before any state is touched, so
        // invalid calls (e.g. an empty stream with a non-positive amount)
        // neither mutate storage nor pay a TTL extension.
        if end_time > 0 && end_time <= start_time {
            return Err(Error::InvalidDuration);
        }
        if deposit <= 0 {
            return Err(Error::InvalidDeposit);
        }
        if rate_per_sec <= 0 {
            return Err(Error::InvalidRate);
        }
        if deposit < rate_per_sec {
            return Err(Error::InsufficientDeposit);
        }
        // Read the ledger clock once: the backdated check and the
        // start-offset bound below must agree on "now", and two reads could
        // straddle a ledger close.
        let now = env.ledger().timestamp();
        if start_time < now {
            return Err(Error::BackdatedStream);
        }
        // A fixed-duration stream must be funded for its entire declared
        // length — otherwise it silently drains before end_time. `deposit
        // >= rate_per_sec` above only guarantees 1 second of streaming.
        if end_time > 0 {
            let duration = (end_time - start_time) as i128;
            let required = rate_per_sec
                .checked_mul(duration)
                .ok_or(Error::ArithmeticOverflow)?;
            if deposit < required {
                return Err(Error::InsufficientDeposit);
            }
        }
        Ok(now)
    }

    #[allow(clippy::too_many_arguments)]
    fn create_stream_with_config(
        env: Env,
        config: &governance::GovernorConfig,
        now: u64,
        sender: Address,
        recipient: Address,
        token: Address,
        deposit: i128,
        rate_per_sec: i128,
        start_time: u64,
        end_time: u64,
        clawback: bool,
    ) -> Result<u64, Error> {
        // ── Governor-controlled bounds ──────────────────────────────────────
        governance::enforce_bounds(config, rate_per_sec, start_time, end_time, now)?;

        // ── Reentrancy guard ─────────────────────────────────────────────
        // `token` is caller-supplied and may not be a well-behaved SEP-41
        // asset. A malicious `transfer` implementation could call back into
        // `create_stream` before this call finishes; the lock (combined with
        // Soroban's all-or-nothing transaction semantics, which roll back
        // every storage write made so far if any nested call returns `Err`)
        // turns a reentrant attempt into a full transaction abort instead of
        // a corrupted `StreamCount`/registry state.
        if env
            .storage()
            .instance()
            .get(&DataKey::CreateLock)
            .unwrap_or(false)
        {
            return Err(Error::CreateLocked);
        }
        env.storage().instance().set(&DataKey::CreateLock, &true);

        // ── All validation passed — safe to touch state now ──────────────
        ttl::bump_instance(&env);

        // ── Pull deposit from sender ──────────────────────────────────────
        // Using the aliased `tok` to avoid any future shadowing issues.
        let tk = tok::Client::new(&env, &token);
        let factory_addr = env.current_contract_address();
        let factory_balance_before = tk.balance(&factory_addr);
        tk.transfer(&sender, &factory_addr, &deposit);
        // Confirm the deposit actually arrived — a non-conforming token
        // could return successfully from `transfer` without moving funds.
        if tk.balance(&factory_addr) != factory_balance_before + deposit {
            env.storage().instance().set(&DataKey::CreateLock, &false);
            return Err(Error::DepositTransferFailed);
        }

        // ── Assign stream ID ─────────────────────────────────────────────
        let stream_count: u64 = env
            .storage()
            .instance()
            .get(&DataKey::StreamCount)
            .unwrap_or(0);
        let stream_id = stream_count;

        let wasm_hash: BytesN<32> = match env.storage().instance().get(&DataKey::StreamWasmHash) {
            Some(hash) => hash,
            None => {
                env.storage().instance().set(&DataKey::CreateLock, &false);
                return Err(Error::NotInitialized);
            }
        };

        // ── Deploy DripStream ────────────────────────────────────────────
        // `config.force_cancel_pause_threshold_seconds` was already read
        // above (governor cross-contract call for bounds enforcement), so
        // passing it into `initialize` here is free — no extra cross-contract
        // call. The deployed stream stores it and reads it locally in
        // `force_cancel`, keeping that contract's hot path free of
        // cross-contract calls per ADR-001.
        let init_args = soroban_sdk::vec![
            &env,
            sender.to_val(),
            recipient.to_val(),
            token.to_val(),
            rate_per_sec.into_val(&env),
            start_time.into_val(&env),
            end_time.into_val(&env),
            clawback.into_val(&env),
            config.force_cancel_pause_threshold_seconds.into_val(&env),
        ];

        let stream_addr = deploy::deploy_stream(&env, &wasm_hash, stream_id, init_args);

        // Forward the deposit into the newly deployed stream contract.
        let stream_balance_before = tk.balance(&stream_addr);
        tk.transfer(&factory_addr, &stream_addr, &deposit);
        if tk.balance(&stream_addr) != stream_balance_before + deposit {
            env.storage().instance().set(&DataKey::CreateLock, &false);
            return Err(Error::StreamFundingFailed);
        }

        // ── Index ─────────────────────────────────────────────────────────
        // StreamAddr and the sender/recipient indices grow without bound, so
        // they use persistent storage (not instance storage) to avoid hitting
        // instance storage size limits as the protocol scales.
        //
        // Persistent storage entry 1 — StreamAddr:
        //   Key:   DataKey::StreamAddr(stream_id)
        //          XDR serialization: [discriminant: u32][stream_id: u64]
        //   Value: Address (the deployed DripStream contract address)
        //          XDR serialization: XDR-encoded contract Address
        env.storage()
            .persistent()
            .set(&DataKey::StreamAddr(stream_id), &stream_addr);
        // Extend TTL on the stream address entry so it outlives ledger pruning.
        env.storage().persistent().extend_ttl(
            &DataKey::StreamAddr(stream_id),
            ttl::THRESHOLD,
            ttl::EXTEND_TO,
        );
        env.storage()
            .instance()
            .set(&DataKey::StreamCount, &(stream_count + 1));

        // Persistent storage entry 2 — BySender (paged):
        //   Key:   DataKey::BySenderPage(sender, page)
        //          XDR serialization: [discriminant: u32][sender: XDR Address][page: u32]
        //   Value: Vec<u64> (ordered list of stream IDs this sender has created)
        //          XDR serialization: XDR-encoded Vec of u64 elements
        index::append_sender_index(&env, &sender, stream_id);

        // Persistent storage entry 3 — ByRecipient (paged):
        //   Key:   DataKey::ByRecipientPage(recipient, page)
        //          XDR serialization: [discriminant: u32][recipient: XDR Address][page: u32]
        //   Value: Vec<u64> (ordered list of stream IDs where this address is recipient)
        //          XDR serialization: XDR-encoded Vec of u64 elements
        index::append_recipient_index(&env, &recipient, stream_id);

        env.storage().instance().set(&DataKey::CreateLock, &false);
        Ok(stream_id)
    }

    /// Create multiple streams in one transaction, all funded and
    /// authorized by the same `sender`.
    ///
    /// Uses the same per-stream validation and deployment path as
    /// `create_stream`; the governor config is fetched once for the whole
    /// batch and threaded through to each stream creation.
    ///
    /// Atomicity: Soroban transactions are all-or-nothing at the host
    /// level. If any request fails validation, the `?` below propagates
    /// that error immediately, and every storage write and token
    /// transfer already made earlier in this same call is rolled back
    /// by the host -- no partial-batch state is ever left behind.
    pub fn create_batch_streams(
        env: Env,
        sender: Address,
        requests: Vec<BatchStreamRequest>,
        clawback: bool,
    ) -> Result<Vec<u64>, Error> {
        if requests.is_empty() {
            return Err(Error::EmptyBatch);
        }
        if requests.len() > MAX_BATCH_SIZE {
            return Err(Error::BatchTooLarge);
        }

        // ── Auth / pause ─────────────────────────────────────────────────
        sender.require_auth();
        if pause::is_paused(&env) {
            return Err(Error::ContractPaused);
        }

        // ── Fetch governor config once for the whole batch ───────────────
        let governor: Address = env
            .storage()
            .instance()
            .get(&DataKey::GovernorAddress)
            .ok_or(Error::NotInitialized)?;
        let config = governance::config(&env, &governor)?;

        let mut stream_ids = Vec::new(&env);
        for request in requests.iter() {
            let now = Self::validate_stream_request(
                &env,
                &sender,
                &request.recipient,
                &request.token,
                request.deposit,
                request.rate_per_sec,
                request.start_time,
                request.end_time,
            )?;
            let stream_id = Self::create_stream_with_config(
                env.clone(),
                &config,
                now,
                sender.clone(),
                request.recipient,
                request.token,
                request.deposit,
                request.rate_per_sec,
                request.start_time,
                request.end_time,
                clawback,
            )?;
            stream_ids.push_back(stream_id);
        }
        Ok(stream_ids)
    }

    /// Returns the deployed contract address for `stream_id`, or `None` if the
    /// ID was never created (or the stream has been archived from storage).
    pub fn stream_address(env: Env, stream_id: u64) -> Option<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::StreamAddr(stream_id))
    }

    /// Permissionlessly advance migration of one sender's legacy index into
    /// paged storage. Returns the number of legacy entries migrated so far.
    pub fn migrate_sender_index(env: Env, sender: Address, max_pages: u32) -> u32 {
        index::migrate_sender_index(&env, sender, max_pages)
    }

    /// Permissionlessly advance migration of one recipient's legacy index into
    /// paged storage. Returns the number of legacy entries migrated so far.
    pub fn migrate_recipient_index(env: Env, recipient: Address, max_pages: u32) -> u32 {
        index::migrate_recipient_index(&env, recipient, max_pages)
    }

    /// Cancel multiple streams in one transaction, all authorized by the
    /// same `sender`.
    ///
    /// Mirrors the bulk-creation ergonomics of [`create_batch_streams`](Self::create_batch_streams)
    /// on the cancellation side. Each stream address in `stream_addresses`
    /// is cancelled via a cross-contract call to `DripStream::cancel`,
    /// reusing the per-stream validation, settlement, and event emission.
    ///
    /// Atomicity: Soroban transactions are all-or-nothing at the host
    /// level. If any cancellation fails (e.g. stream already cancelled,
    /// sender mismatch), the `?` below propagates that error immediately,
    /// and every state change already made earlier in this same call is
    /// rolled back by the host — no partial-batch state is ever left behind.
    pub fn cancel_batch_streams(
        env: Env,
        sender: Address,
        stream_addresses: Vec<Address>,
    ) -> Result<(), Error> {
        sender.require_auth();

        if stream_addresses.is_empty() {
            return Err(Error::EmptyBatch);
        }
        if stream_addresses.len() > MAX_BATCH_SIZE {
            return Err(Error::BatchTooLarge);
        }

        // Deduplicate addresses to prevent attempting multiple cancels on the same stream.
        // Issue #416: If a duplicated address is passed, the first cancel succeeds
        // and sets FLAG_CANCELLED; the second cancel on the now-cancelled stream would
        // return Error::StreamCancelled, which the non-try_ variant turns into a panic.
        // Deduplicating the list ensures each unique stream is cancelled exactly once.
        let mut unique_addresses: Vec<Address> = Vec::new(&env);
        for stream_addr in stream_addresses.iter() {
            let mut already_seen = false;
            for seen_addr in unique_addresses.iter() {
                if stream_addr == seen_addr {
                    already_seen = true;
                    break;
                }
            }
            if !already_seen {
                unique_addresses.push_back(stream_addr);
            }
        }

        for stream_addr in unique_addresses.iter() {
            let stream_client = drip_stream::DripStreamClient::new(&env, &stream_addr);
            stream_client.cancel(&sender);
        }

        Ok(())
    }

    /// Batch-resolve stream IDs to their deployed contract addresses.
    ///
    /// Pairs with `streams_by_sender`/`streams_by_recipient`: a page of IDs
    /// from either can be resolved to addresses in one call instead of one
    /// `stream_address` round-trip per ID. Unknown IDs resolve to `None` in
    /// their slot, matching `stream_address`'s per-ID behavior, rather than
    /// failing the whole batch. Capped at `MAX_BATCH_SIZE`, mirroring
    /// `create_batch_streams`.
    pub fn stream_addresses(env: Env, ids: Vec<u64>) -> Result<Vec<Option<Address>>, Error> {
        if ids.len() > MAX_BATCH_SIZE {
            return Err(Error::BatchTooLarge);
        }
        let mut out = Vec::new(&env);
        for id in ids.iter() {
            out.push_back(Self::stream_address(env.clone(), id));
        }
        Ok(out)
    }

    /// Paginated list of stream IDs created by `sender`, paired with the
    /// sender's total stream count.
    ///
    /// Returns at most `limit` IDs starting at `offset`, capped at
    /// [`query::MAX_PAGE_SIZE`] (100) regardless of how large `limit` is —
    /// this cap is silent, so compare `offset + result.ids.len()` against
    /// `result.total` to tell "capped" apart from "sender has no more
    /// streams" instead of guessing from `ids.len()` alone or issuing a
    /// separate `stream_count_by_sender` call. When `offset` exceeds the
    /// total count, `ids` is empty (no error) and `total` still reports the
    /// real count.
    pub fn streams_by_sender(env: Env, sender: Address, offset: u32, limit: u32) -> StreamPage {
        index::streams_by_sender(&env, sender, offset, limit)
    }

    /// Paginated list of stream IDs where `recipient` is the beneficiary,
    /// paired with the recipient's total stream count.
    ///
    /// Returns at most `limit` IDs starting at `offset`, capped at
    /// [`query::MAX_PAGE_SIZE`] (100) regardless of how large `limit` is —
    /// this cap is silent, so compare `offset + result.ids.len()` against
    /// `result.total` to tell "capped" apart from "recipient has no more
    /// streams" instead of guessing from `ids.len()` alone or issuing a
    /// separate `stream_count_by_recipient` call. When `offset` exceeds the
    /// total count, `ids` is empty (no error) and `total` still reports the
    /// real count.
    pub fn streams_by_recipient(
        env: Env,
        recipient: Address,
        offset: u32,
        limit: u32,
    ) -> StreamPage {
        index::streams_by_recipient(&env, recipient, offset, limit)
    }

    /// Total number of streams created by `sender`.
    ///
    /// Mirrors the global `stream_count` but scoped to one sender, so clients
    /// can size pagination UI without walking pages to discover the total.
    pub fn stream_count_by_sender(env: Env, sender: Address) -> u32 {
        index::stream_count_by_sender(&env, sender)
    }

    /// Total number of streams where `recipient` is the beneficiary.
    pub fn stream_count_by_recipient(env: Env, recipient: Address) -> u32 {
        index::stream_count_by_recipient(&env, recipient)
    }

    /// Total number of streams ever created by this factory.
    pub fn stream_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::StreamCount)
            .unwrap_or(0)
    }

    /// Read-only: current protocol fee in basis points.
    ///
    /// Reads live from DripGovernor.
    ///
    /// # Errors
    ///
    /// - `NotInitialized` — the factory has no governor address yet, so there
    ///   is nothing to read a fee from.
    /// - `GovernorNotResponding` — the governor is archived, uninitialised, or
    ///   the cross-contract call failed.
    ///
    /// Both cases previously returned a hardcoded `30`, which a caller could
    /// not distinguish from a governor genuinely configured at 30 bps. That is
    /// the same situation in which `create_stream` fails loudly with
    /// `GovernorNotResponding`, so a UI quoting a fee and a transaction
    /// charging one could disagree without anything appearing to go wrong.
    /// Callers that want the old lenient behaviour should use
    /// [`Self::protocol_fee_bps_or_default`].
    pub fn protocol_fee_bps(env: Env) -> Result<u32, Error> {
        let governor: Option<Address> = env.storage().instance().get(&DataKey::GovernorAddress);
        let governor = governor.ok_or(Error::NotInitialized)?;
        governance::config(&env, &governor).map(|c| c.fee_bps)
    }

    /// Read-only: current protocol fee, falling back to `default_bps` when it
    /// cannot be read.
    ///
    /// For callers that would rather display an approximate fee than nothing —
    /// but which are choosing that tradeoff explicitly, and supply the fallback
    /// themselves rather than inheriting a constant buried in the factory.
    pub fn protocol_fee_bps_or_default(env: Env, default_bps: u32) -> u32 {
        Self::protocol_fee_bps(env).unwrap_or(default_bps)
    }

    /// Update the stored stream WASM hash.
    ///
    /// Called after a new stream contract version is uploaded so subsequent
    /// `create_stream` calls deploy the new implementation. Existing streams
    /// are unaffected — each is an independent deployed contract.
    ///
    /// As a maintenance operation, this also drives the bounded TTL walker
    /// so the persistent-by-ID registry stays alive during protocol-upgrade
    /// activity even if no `create_stream` calls happen between upgrades.
    pub fn upgrade_stream_wasm(env: Env, new_wasm_hash: BytesN<32>) -> Result<(), Error> {
        // Only governor may update the wasm hash.
        let governor: Address = env
            .storage()
            .instance()
            .get(&DataKey::GovernorAddress)
            .ok_or(Error::NotInitialized)?;
        governor.require_auth();

        // ── Boundary / null checks ───────────────────────────────────────
        // Reject all-zero WASM hashes — deploying with a zero hash would
        // deploy a no-op contract, effectively burning all funds sent to
        // future `create_stream` calls with no way to recover them.
        if is_zero_wasm_hash(&env, &new_wasm_hash) {
            return Err(Error::InvalidWasmHash);
        }

        // ── Lifecycle check ──────────────────────────────────────────────
        // Block upgrades while the factory is under an emergency pause.
        // A paused factory should accept no state mutations at all, even
        // from the governor, so the halt remains comprehensive.
        if pause::is_paused(&env) {
            return Err(Error::ContractPaused);
        }

        ttl::bump_instance(&env);
        ttl::bump_persistent_bucket(&env);
        env.storage()
            .instance()
            .set(&DataKey::StreamWasmHash, &new_wasm_hash);
        Ok(())
    }

    /// Replace this contract's own WASM bytecode.
    ///
    /// The new WASM must already be uploaded to the ledger (via
    /// `stellar contract upload`); only the hash is passed here. Gated on
    /// the governor, matching `upgrade_stream_wasm` — the same authority
    /// that controls protocol parameters controls code changes.
    ///
    /// This is distinct from `upgrade_stream_wasm`, which only updates the
    /// WASM hash used for *future* `create_stream` deployments. `upgrade`
    /// replaces the factory's own implementation.
    ///
    /// `expected_storage_version` must equal the *currently stored*
    /// `DataKey::FactoryStorageVersion` (readable via `storage_version()`).
    /// Upgrade tooling should read `storage_version()` and the new WASM's
    /// own `storage::CURRENT_STORAGE_VERSION` before submitting this call,
    /// and pass the value it confirmed matches — this guards against a
    /// storage-layout change being deployed onto existing state without an
    /// explicit migration step, mirroring `DripStream::storage_version`.
    pub fn upgrade(
        env: Env,
        new_wasm_hash: BytesN<32>,
        expected_storage_version: u32,
    ) -> Result<(), Error> {
        let governor: Address = env
            .storage()
            .instance()
            .get(&DataKey::GovernorAddress)
            .ok_or(Error::NotInitialized)?;
        governor.require_auth();

        if is_zero_wasm_hash(&env, &new_wasm_hash) {
            return Err(Error::InvalidWasmHash);
        }

        if pause::is_paused(&env) {
            return Err(Error::ContractPaused);
        }

        let stored_version: u32 = env
            .storage()
            .instance()
            .get(&DataKey::FactoryStorageVersion)
            .ok_or(Error::NotInitialized)?;
        if expected_storage_version != stored_version {
            return Err(Error::StorageVersionMismatch);
        }

        ttl::bump_instance(&env);
        env.deployer().update_current_contract_wasm(new_wasm_hash);
        events::upgraded(&env, &governor, env.ledger().timestamp());
        Ok(())
    }

    /// Storage layout version this instance was initialized with.
    ///
    /// Upgrade tooling should read this before calling `upgrade` and confirm
    /// it matches both the value passed as `expected_storage_version` and
    /// the new WASM's own expected version. Mirrors
    /// `DripStream::storage_version`.
    pub fn storage_version(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::FactoryStorageVersion)
            .unwrap_or(0)
    }

    /// Emergency halt: stop all new stream creation.
    ///
    /// Intended for an extreme protocol emergency. While paused, every
    /// `create_stream` call reverts with `ContractPaused` before any deposit
    /// is pulled. Existing streams are independent deployed contracts and keep
    /// running; front-ends and the stream contract can gate withdrawals by
    /// reading `is_paused`.
    ///
    /// Gated on the governor, matching `upgrade_stream_wasm` — the same
    /// authority that controls protocol parameters controls the halt.
    pub fn pause(env: Env) -> Result<(), Error> {
        let governor: Address = env
            .storage()
            .instance()
            .get(&DataKey::GovernorAddress)
            .ok_or(Error::NotInitialized)?;
        governor.require_auth();
        if pause::is_paused(&env) {
            return Err(Error::AlreadyPaused);
        }
        ttl::bump_instance(&env);
        // Maintenance op — drive the bounded TTL walker so the persistent
        // registry doesn't silently archive during an emergency-pause idle
        // period.
        ttl::bump_persistent_bucket(&env);
        pause::set_paused(&env, true);
        // Emit a positive signal of the transition so off-chain indexers and
        // relayers can confirm the halt committed (see `events::paused`),
        // rather than inferring it from a bare `Ok` that a dropped or
        // rate-limited RPC response may have lost.
        events::paused(&env, &governor, env.ledger().timestamp());
        Ok(())
    }

    /// Lift the emergency pause, allowing `create_stream` again.
    ///
    /// Gated on the governor, matching `pause`.
    pub fn unpause(env: Env) -> Result<(), Error> {
        let governor: Address = env
            .storage()
            .instance()
            .get(&DataKey::GovernorAddress)
            .ok_or(Error::NotInitialized)?;
        governor.require_auth();
        if !pause::is_paused(&env) {
            return Err(Error::NotPaused);
        }
        ttl::bump_instance(&env);
        // Maintenance op — drive the bounded TTL walker so the persistent
        // registry doesn't silently archive while the protocol is coming
        // back online and `create_stream` hasn't yet resumed.
        ttl::bump_persistent_bucket(&env);
        pause::set_paused(&env, false);
        // Emit a positive signal of the transition so off-chain infra can
        // confirm creation resumed (see `events::unpaused`).
        events::unpaused(&env, &governor, env.ledger().timestamp());
        Ok(())
    }

    /// Read-only: whether the factory is currently under an emergency pause.
    ///
    /// Returns `false` for a factory that predates this feature (the flag was
    /// never written). Exposed so the stream contract and off-chain infra can
    /// enforce the halt on withdrawals as well as creation.
    pub fn is_paused(env: Env) -> bool {
        pause::is_paused(&env)
    }

    /// Read-only: combined factory status (pause state and protocol fee bps).
    ///
    /// Combines `is_paused` and `protocol_fee_bps` into a single view call
    /// to save a round-trip for UI/indexer health checks.
    ///
    /// `protocol_fee_bps` is `None` when the fee could not be read. The call
    /// still succeeds in that case, because the pause state is independently
    /// useful and a governor outage should not hide it.
    pub fn factory_status(env: Env) -> FactoryStatus {
        FactoryStatus {
            is_paused: Self::is_paused(env.clone()),
            protocol_fee_bps: Self::protocol_fee_bps(env).ok(),
        }
    }

    /// Estimate the Soroban resource cost for a given stream operation.
    ///
    /// Returns a [`FeeEstimate`] with the operation's expected CPU
    /// instructions and ledger entry counts. The actual network fee in
    /// stroops is computed by the frontend using `simulateTransaction`,
    /// which returns the exact cost from the current network base fee.
    ///
    /// This is a read-only call — no state is modified, no auth is required.
    pub fn estimate_fee(_env: Env, operation: StreamOperation) -> FeeEstimate {
        // Resource costs are deterministic per operation type and derived
        // from profiling the Soroban host execution of each operation path.
        //
        // Each operation has a distinct cost profile determined by the
        // number of host objects created, storage reads/writes, and
        // cross-contract invocations it performs.
        let (cpu_instructions, ledger_entries) = match operation {
            StreamOperation::CreateStream => {
                // Highest cost: contract deployment (WASM instantiate),
                // governor cross-contract config call, 3 persistent storage
                // writes (StreamAddr, BySender, ByRecipient), 2 token
                // transfers, instance storage TTL bumps, and event emission.
                //
                // CPU: ~2_500_000 instructions (deploy + initialize + index)
                // Entries: 1 WASM hash + 3 persistent + 1 instance = 5
                (2_500_000, 5)
            }
            StreamOperation::CancelStream => {
                // Moderate cost: single cross-contract call to DripStream::cancel,
                // token transfer back to sender, event emission.
                //
                // CPU: ~800_000 instructions
                // Entries: 1 read (stream info) + 1 write (cancelled flag) = 2
                (800_000, 2)
            }
            StreamOperation::Withdraw => {
                // Lowest cost: single cross-contract call to DripStream::withdraw,
                // token transfer, event emission.
                //
                // CPU: ~500_000 instructions
                // Entries: 1 read (stream info) = 1
                (500_000, 1)
            }
            StreamOperation::PauseStream => {
                // Low cost: single cross-contract call, storage write.
                //
                // CPU: ~400_000 instructions
                // Entries: 1 read + 1 write = 2
                (400_000, 2)
            }
            StreamOperation::ResumeStream => {
                // Low cost: single cross-contract call, storage write.
                //
                // CPU: ~400_000 instructions
                // Entries: 1 read + 1 write = 2
                (400_000, 2)
            }
        };

        // fee_stroops and fee_xlm are set to 0 here — the frontend
        // computes the actual fee via RPC simulateTransaction, which
        // returns the real network cost from the current base fee.
        FeeEstimate {
            fee_stroops: 0,
            fee_xlm: 0,
            cpu_instructions,
            ledger_entries,
        }
    }
}
