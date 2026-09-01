#![no_std]

//! Shared constants and utilities for the Drip protocol contracts.

pub mod rbac;

use soroban_sdk::{Address, Env};

/// TTL threshold for instance storage extension.
/// When the remaining TTL falls below this value, extend to `TTL_EXTEND_TO`.
pub const TTL_THRESHOLD: u32 = 100_000;

/// Target TTL for instance storage extension.
/// Extended to this value when `TTL_THRESHOLD` is reached.
pub const TTL_EXTEND_TO: u32 = 200_000;

/// Returns true when `address` is a known zero/degenerate address.
///
/// Soroban's `Address` (protocol 21) can only ever wrap an XDR `ScAddress`
/// of variant `Account` or `Contract` — there is no separate muxed-address
/// representation at this layer — so checking both zero forms below covers
/// every degenerate address the host can construct:
///
/// - the all-zero Ed25519 account (`G...` strkey `GAAAA...AWHF`), and
/// - the all-zero Soroban contract address (`C...` strkey `CAAAA...BSC4`).
///
/// Both literals are hardcoded here once so every contract sharing this
/// helper uses the exact same values — a duplicated copy would be easy to
/// typo differently without anyone noticing. The account form is checked
/// first since it is the more common case, so a real account address never
/// pays for parsing the contract literal.
pub fn is_zero_address(env: &Env, address: &Address) -> bool {
    let zero_account = Address::from_string(&soroban_sdk::String::from_str(
        env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    ));
    if address == &zero_account {
        return true;
    }

    let zero_contract = Address::from_string(&soroban_sdk::String::from_str(
        env,
        "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4",
    ));

    address == &zero_contract
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn rejects_zero_ed25519_account() {
        let env = Env::default();
        let zero_account = Address::from_string(&soroban_sdk::String::from_str(
            &env,
            "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
        ));
        assert!(is_zero_address(&env, &zero_account));
    }

    #[test]
    fn rejects_zero_contract_address() {
        let env = Env::default();
        let zero_contract = Address::from_string(&soroban_sdk::String::from_str(
            &env,
            "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4",
        ));
        assert!(is_zero_address(&env, &zero_contract));
    }

    #[test]
    fn accepts_generated_address() {
        let env = Env::default();
        let address = Address::generate(&env);
        assert!(!is_zero_address(&env, &address));
    }
}
