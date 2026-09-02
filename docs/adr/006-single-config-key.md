# ADR-006: Single-Key Storage Consolidation

**Status:** Accepted  
**Date:** 2026-03

---

## Context

`DripStream` originally stored each `StreamInfo` field under a separate `DataKey`:

- `Sender`, `Recipient`, `Token`
- `RatePerSecond`, `StartTime`, `EndTime`
- `Withdrawn`, `PausedAt`
- `Flags`, `ClawbackEnabled`, `Cancelled`
- `EventSequence`

That is 12 independent instance-storage keys. Every state-mutating call (`withdraw`, `pause`, `resume`, `cancel`, `top_up`, `extend_duration`) had to write back all 12 keys to keep them consistent, even though most fields never changed in that transition.

## Decision

Replace the 12 per-field keys with a single `Config` key that holds the entire `StreamInfo` struct. Reads and writes now touch exactly one storage entry.

## Rationale

**Cost.** On Soroban, each `instance().set()` call has a base cost plus per-byte rent. Writing one consolidated struct is cheaper than writing 12 small entries, especially because the overhead of 11 extra key-value pairs dwarfed the actual data size.

**Atomicity.** A single-key write is inherently atomic — there is no window where `Withdrawn` has been updated but `PausedAt` has not. With 12 keys, a re-entrancy or panic between the 3rd and 4th write could leave the stream in an inconsistent state (mitigated by the re-entrancy guard, but still a risk surface).

**Code simplicity.** `state::load()` and `state::save()` are now one-liners around `Config`. The old 12-key read/write logic lives only in the legacy fallback path.

## Migration story

No on-chain migration is required. The change is backward-compatible via a two-path `load()`:

1. **Fast path:** try `DataKey::Config` first. All streams initialized after this ADR write `Config` at `initialize()`.
2. **Legacy path:** if `Config` is absent, read the 12 individual keys and reconstruct the `flags` bitfield from the old `ClawbackEnabled` / `Cancelled` booleans plus the existing `Flags` key.

The first `save()` on a legacy stream performs a one-time cleanup:

- Write `Config` with the full `StreamInfo`.
- Remove all 12 legacy keys from instance storage.

After that first post-consolidation mutation, the stream behaves exactly like a newly created one — one read, one write.

## Trade-offs

- **Larger single read.** `Config` reads ~200 bytes in one go instead of 12 small reads. This is still cheaper than the combined overhead of 12 key lookups.
- **No partial updates.** Even if only `Withdrawn` changes, the whole `StreamInfo` is rewritten. On Soroban instance storage this is acceptable because the struct is small and the write cost is dominated by the key overhead we eliminated.
- **No cross-field atomicity issues.** Because there is only one key, we removed an entire class of partial-write bugs.

## Consequences

- All new streams use the single-key layout from birth.
- Legacy streams transparently migrate on their next state change.
- The 12 legacy `DataKey` variants remain in the enum for the fallback path but are never written by new code.
- SDK and indexer authors only need to understand `StreamInfo` — the individual keys are an implementation detail.
