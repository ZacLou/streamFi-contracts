#![no_std]

use soroban_sdk::{contract, contracterror, contractimpl, token, Address, Env, Vec};

/// Maximum number of transfers permitted in a single batch.
const MAX_BATCH_SIZE: u32 = 100;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// `recipients` and `amounts` have different lengths.
    LengthMismatch = 1,
    /// The batch exceeds [`MAX_BATCH_SIZE`].
    BatchTooLarge = 2,
    /// An individual amount is zero or negative.
    InvalidAmount = 3,
    /// Integer overflow computing the total to pull from the funder.
    ArithmeticOverflow = 4,
}

#[contract]
pub struct BatchTransferProcessor;

#[contractimpl]
impl BatchTransferProcessor {
    /// Transfer tokens from `funder` to each address in `recipients`.
    ///
    /// # Auth
    /// `funder.require_auth()` is called before any state mutation.  The funder
    /// must have pre-authorised a transfer of at least `sum(amounts)` tokens to
    /// this contract, which then fans the funds out to the recipients in order.
    ///
    /// # Validation (all checks precede any token movement)
    /// - `recipients` and `amounts` must have the same length.
    /// - The batch must not exceed `MAX_BATCH_SIZE` (100 entries).
    /// - Every individual `amount` must be > 0.
    ///
    /// # Returns
    /// The total number of tokens transferred on success.
    pub fn process_batch(
        env: Env,
        funder: Address,
        token: Address,
        recipients: Vec<Address>,
        amounts: Vec<i128>,
    ) -> Result<i128, Error> {
        // ── Input validation (before auth and any token movement) ────────────

        if recipients.len() != amounts.len() {
            return Err(Error::LengthMismatch);
        }

        if amounts.len() > MAX_BATCH_SIZE {
            return Err(Error::BatchTooLarge);
        }

        // Validate every amount and accumulate the total in one pass so we
        // pull a single lump sum from the funder instead of N separate auths.
        let mut total: i128 = 0;
        for amount in amounts.iter() {
            if amount <= 0 {
                return Err(Error::InvalidAmount);
            }
            total = total.checked_add(amount).ok_or(Error::ArithmeticOverflow)?;
        }

        // Empty batch — nothing to do.
        if total == 0 {
            return Ok(0);
        }

        // ── Auth ─────────────────────────────────────────────────────────────
        funder.require_auth();

        // ── Token transfers ───────────────────────────────────────────────────
        let tk = token::Client::new(&env, &token);
        let contract_addr = env.current_contract_address();

        // Pull the full batch total from the funder into this contract in one
        // transfer, then fan out to each recipient individually.  One inbound
        // transfer keeps the auth surface minimal (funder signs once).
        tk.transfer(&funder, &contract_addr, &total);

        for (recipient, amount) in recipients.iter().zip(amounts.iter()) {
            tk.transfer(&contract_addr, &recipient, &amount);
        }

        Ok(total)
    }
}
