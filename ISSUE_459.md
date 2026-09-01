# Issue #459

## Title
Factory paged-index offset math is easy to get subtly wrong.

## Summary
The factory paged index mixes legacy cursor metadata with logical offsets and page-size boundaries. A small arithmetic mistake can silently move reads or appends to the wrong page or offset, especially when a legacy cursor is non-zero and the logical index has crossed into the newly appended pages.

## Risk
This is subtle because the code appears correct at a glance, but the combination of:
- legacy_count
- legacy cursor
- logical offset
- page size (100)

can produce off-by-one or page-boundary errors when the index spans legacy and paged storage.

## Desired invariant
For any valid triple of `(legacy_count, cursor, logical)`:
- the computed physical offset must be monotonic as `logical` increases
- the physical offset must remain in range for the current index layout
- when `logical` is still in the legacy portion, the offset should remain the logical offset
- when `logical` crosses into appended pages, the offset should advance by the legacy-page alignment offset

## Suggested validation
Add property-based tests over `(legacy_count, cursor, logical)` triples checking that the physical offset:
- is monotonic for increasing logical values
- stays within the range of the migrated/paged index
- never moves backward when the cursor advances
- remains consistent with the page-size boundary math

## Notes
This issue is specifically about the subtle arithmetic around the legacy cursor plus page-size migration math, and should be protected by proptest coverage to prevent regressions.
