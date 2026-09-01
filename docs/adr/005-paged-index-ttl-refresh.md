# ADR-005: Refresh paged-index TTL on every read

**Status:** Accepted  
**Date:** 2026-09

---

## Context

The factory keeps per-user stream indices in paged persistent storage. Each page holds at most 50 stream IDs. New entries are appended to the current tail page; once a page fills it is never written again. Queries, however, tend to read only the most recent window ("latest first"), so older pages would naturally age toward archival unless something refreshes their TTL.

Soroban persistent storage entries that fall below the TTL threshold are archived. An archived page is still recoverable, but it becomes invisible to normal contract reads and would silently truncate query results relative to the stored count.

## Decision

Every call to `read_index` refreshes the TTL of *all* populated pages, not only the pages touched by the current query window. The refresh is amortised: `extend_page_ttls` walks at most three pages per call, using the current ledger sequence to pseudo-randomly distribute the work over time.

## Rationale

**Archival safety.** A page that has filled and is no longer written would only survive if a query happened to land in its range. Because UIs typically paginate from the newest entries, page 0 (the oldest) would archive first, making the total count disagree with the returned IDs.

**Bounded per-read cost.** Refreshing every page on every read would be O(N). By capping the walk at three pages and choosing the starting point with `ledger_sequence % num_pages`, the cost stays constant while every page is touched regularly as the ledger advances.

**Complementary append-time refresh.** `append_index_entry` already extends the TTL of the page it writes and calls `extend_page_ttls` after insertion. The read-time refresh is a backstop for pages that are no longer appended to.

**Index metadata is also refreshed.** `read_index` extends the TTL of `count_key`, `legacy_count_key`, `cursor_key`, and the legacy index entry while it is still present, keeping migration state alive.

## Consequences

- **Higher per-read CPU cost.** Every `streams_by_sender` / `streams_by_recipient` query pays for up to three additional `extend_ttl` calls regardless of the query window.
- **No silent truncation.** Query results remain consistent with the stored count as long as reads occur with reasonable frequency.
- **No explicit archival recovery path required.** The protocol does not need to handle re-hydrating archived index pages during normal reads.
- **TTL constants remain authoritative.** `ttl::THRESHOLD` and `ttl::EXTEND_TO` still control the refresh cadence; this decision only spreads the refresh work across reads.

## Related

- Implementation: `contracts/factory/src/index.rs`
- Original fix discussion: #401
