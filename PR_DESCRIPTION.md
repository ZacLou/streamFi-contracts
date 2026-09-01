# Add Comprehensive Tests for feeBps Validation (100% Cap)

## Issue
Closes: Reject governor feeBps > 10000 (100%)

## Summary
This PR adds comprehensive test coverage for the existing `feeBps` validation that prevents protocol fees from exceeding 100% (10,000 basis points).

## Changes Made
- ✅ Added `tests.rs` module to `contracts/governor/src/`
- ✅ Created 7 test cases covering:
  - Valid fee values (0, 100, 5000, 10000)
  - Invalid values over 10,000 are rejected
  - Boundary conditions (10,000 passes, 10,001 fails)
  - Event emission verification
  - Role-based access control (FeeManager required)
  - Pause state enforcement
- ✅ Integrated tests module into `lib.rs`

## Validation Logic (Already Exists)
The validation in `set_fee_bps` function (line 414-416 of `lib.rs`):
```rust
if fee_bps > 10_000 {
    return Err(Error::InvalidParam);
}
```

This ensures no mis-configured value can take the entire stream as protocol fee.

## Test Coverage
1. **test_set_fee_bps_accepts_valid_values** - Verifies 0, 100, 5000, and 10,000 are accepted
2. **test_set_fee_bps_rejects_values_over_10000** - Verifies 10,001, 15,000, 100,000, and u32::MAX are rejected
3. **test_set_fee_bps_boundary_values** - Specifically tests the 10,000/10,001 boundary
4. **test_set_fee_bps_emits_event** - Verifies proper event emission
5. **test_set_fee_bps_requires_fee_manager_role** - Ensures unauthorized users cannot set fees
6. **test_set_fee_bps_blocked_when_paused** - Confirms fees cannot be changed when contract is paused

## Security Impact
These tests provide confidence that the 100% fee cap is enforced correctly, preventing:
- Accidental misconfiguration taking 100%+ of streams
- Malicious attempts to set excessive fees
- Edge cases at boundary values

## Notes
The actual validation logic already existed in the codebase. This PR adds the missing test coverage to ensure the security guarantee is maintained through future changes.
