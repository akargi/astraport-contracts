//! # AstraPort Payments Contract
//!
//! The protocol's escrow / settlement layer: real Soroban token deposits,
//! withdrawals and internal transfers, two-party escrow with release/refund,
//! admin role rotation, and per-asset / global spending caps.
//!
//! ## Design notes
//!
//! - **Real tokens.** Every money-moving entry point performs a
//!   `soroban_sdk::token::Client` transfer. Internal balances are a mirror of
//!   tokens actually held by this contract, never created from thin air.
//! - **Check-effects-then-transfer.** Internal ledgers are finalized before any
//!   outbound token transfer, so a failing transfer can never leave partial
//!   state (Soroban reverts the whole invocation on failure, but ordering still
//!   keeps the reasoning simple and matches the staking contract's style).
//! - **Typed errors, no panics.** Public entry points return
//!   `Result<_, Error>`; `initialize` is one-time via a typed error.
//! - **Caps.** A global per-transaction cap and per-asset caps bound the size
//!   of any single outflow (withdraw, transfer, escrow create/release),
//!   mirroring the emergency contract's `max_trade_amount` guard.
//!
//! ## Module overview
//!
//! - [`records`] — storage keys, escrow records, events, and the error enum.

#![no_std]

use soroban_sdk::{
    contract, contractimpl, symbol_short, token::Client as TokenClient, Address, Env, Symbol, Vec,
};

pub mod records;

#[cfg(test)]
mod tests;

use records::{
    AdminSetEvent, CapSetEvent, DepositEvent, Error, EscrowCreatedEvent, EscrowRecord,
    EscrowSettledEvent, EscrowStatus, PaymentsDataKey, TransferEvent, WithdrawEvent,
    MAX_ESCROWS_RETURN,
};

/// Sentinel return symbol.
const OK: Symbol = symbol_short!("ok");

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn get_admin(env: &Env) -> Result<Address, Error> {
    env.storage()
        .persistent()
        .get(&PaymentsDataKey::Admin)
        .ok_or(Error::NotInitialized)
}

fn assert_admin(env: &Env, caller: &Address) -> Result<(), Error> {
    let admin = get_admin(env)?;
    if *caller != admin {
        return Err(Error::Unauthorized);
    }
    Ok(())
}

fn token_address(env: &Env, asset: &Symbol) -> Result<Address, Error> {
    env.storage()
        .persistent()
        .get(&PaymentsDataKey::TokenMap(asset.clone()))
        .ok_or(Error::TokenNotRegistered)
}

fn get_balance(env: &Env, owner: &Address, asset: &Symbol) -> i128 {
    env.storage()
        .persistent()
        .get(&PaymentsDataKey::Balance(owner.clone(), asset.clone()))
        .unwrap_or(0)
}

fn put_balance(env: &Env, owner: &Address, asset: &Symbol, amount: i128) {
    env.storage().persistent().set(
        &PaymentsDataKey::Balance(owner.clone(), asset.clone()),
        &amount,
    );
}

/// Effective per-transaction cap for `asset`: the smaller of the global cap
/// (if set) and the asset cap (if set). `None` means unlimited.
fn effective_cap(env: &Env, asset: &Symbol) -> Option<i128> {
    let global: Option<i128> = env.storage().persistent().get(&PaymentsDataKey::GlobalCap);
    let per_asset: Option<i128> = env
        .storage()
        .persistent()
        .get(&PaymentsDataKey::AssetCap(asset.clone()));

    match (global, per_asset) {
        (Some(g), Some(a)) => Some(g.min(a)),
        (Some(g), None) => Some(g),
        (None, Some(a)) => Some(a),
        (None, None) => None,
    }
}

fn check_cap(env: &Env, asset: &Symbol, amount: i128) -> Result<(), Error> {
    if let Some(cap) = effective_cap(env, asset) {
        if amount > cap {
            return Err(Error::CapExceeded);
        }
    }
    Ok(())
}

/// Debit `amount` from `balance`, failing closed when the result would be
/// negative.
///
/// NOTE: `checked_sub` alone is NOT sufficient — it only catches crossing
/// `i128::MIN`, and `balance - amount` may legitimately be a small negative
/// number. Balances must never go below zero.
fn checked_debit(balance: i128, amount: i128) -> Result<i128, Error> {
    let new_balance = balance
        .checked_sub(amount)
        .ok_or(Error::ArithmeticOverflow)?;
    if new_balance < 0 {
        return Err(Error::InsufficientBalance);
    }
    Ok(new_balance)
}

fn emit_deposit(env: &Env, asset: &Symbol, from: &Address, amount: i128, new_balance: i128) {
    env.events().publish(
        (symbol_short!("DEPOSIT"), asset.clone()),
        DepositEvent {
            asset: asset.clone(),
            from: from.clone(),
            amount,
            new_balance,
        },
    );
}

fn emit_withdraw(env: &Env, asset: &Symbol, to: &Address, amount: i128, new_balance: i128) {
    env.events().publish(
        (symbol_short!("WITHDRAW"), asset.clone()),
        WithdrawEvent {
            asset: asset.clone(),
            to: to.clone(),
            amount,
            new_balance,
        },
    );
}

fn emit_transfer(
    env: &Env,
    asset: &Symbol,
    from: &Address,
    to: &Address,
    amount: i128,
    from_balance: i128,
    to_balance: i128,
) {
    env.events().publish(
        (symbol_short!("TRANSFER"), asset.clone()),
        TransferEvent {
            asset: asset.clone(),
            from: from.clone(),
            to: to.clone(),
            amount,
            from_balance,
            to_balance,
        },
    );
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

/// AstraPort Payments — escrow / settlement layer with real token custody.
#[contract]
pub struct PaymentsContract;

#[contractimpl]
impl PaymentsContract {
    // =======================================================================
    // Lifecycle
    // =======================================================================

    /// Initialize the payments contract with an admin.
    ///
    /// Can only be called once; subsequent calls return
    /// [`Error::AlreadyInitialized`] (never panics).
    pub fn initialize(env: Env, admin: Address) -> Result<Symbol, Error> {
        let storage = env.storage().persistent();
        if storage.has(&PaymentsDataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        storage.set(&PaymentsDataKey::Admin, &admin);
        Ok(OK)
    }

    /// Transfer the admin role to `new_admin`. Only the current admin may call.
    pub fn transfer_admin(env: Env, caller: Address, new_admin: Address) -> Result<Symbol, Error> {
        caller.require_auth();
        let old_admin = get_admin(&env)?;
        assert_admin(&env, &caller)?;

        env.storage()
            .persistent()
            .set(&PaymentsDataKey::Admin, &new_admin);

        env.events().publish(
            (symbol_short!("ADMIN_SET"), new_admin.clone()),
            AdminSetEvent {
                old_admin,
                new_admin,
            },
        );
        Ok(OK)
    }

    /// Get the current admin, if initialized.
    pub fn get_admin(env: Env) -> Result<Address, Error> {
        get_admin(&env)
    }

    // =======================================================================
    // Token registration (admin)
    // =======================================================================

    /// Register (or replace) the token contract backing `asset`.
    pub fn set_token(
        env: Env,
        admin: Address,
        asset: Symbol,
        token_id: Address,
    ) -> Result<Symbol, Error> {
        admin.require_auth();
        assert_admin(&env, &admin)?;

        env.storage()
            .persistent()
            .set(&PaymentsDataKey::TokenMap(asset.clone()), &token_id);
        Ok(OK)
    }

    /// Get the registered token contract for `asset`.
    pub fn get_token(env: Env, asset: Symbol) -> Result<Address, Error> {
        token_address(&env, &asset)
    }

    // =======================================================================
    // Spending caps (admin)
    // =======================================================================

    /// Set the global per-transaction cap. `cap == 0` disables the global cap.
    pub fn set_global_cap(env: Env, admin: Address, cap: i128) -> Result<Symbol, Error> {
        admin.require_auth();
        assert_admin(&env, &admin)?;
        if cap < 0 {
            return Err(Error::InvalidAmount);
        }

        let cap_opt = if cap == 0 { None } else { Some(cap) };
        env.storage()
            .persistent()
            .set(&PaymentsDataKey::GlobalCap, &cap_opt);

        env.events().publish(
            (symbol_short!("CAP_SET"), records::global_asset_symbol()),
            CapSetEvent {
                asset: records::global_asset_symbol(),
                cap,
            },
        );
        Ok(OK)
    }

    /// Set the per-transaction cap for `asset`. `cap == 0` removes the asset cap.
    pub fn set_asset_cap(
        env: Env,
        admin: Address,
        asset: Symbol,
        cap: i128,
    ) -> Result<Symbol, Error> {
        admin.require_auth();
        assert_admin(&env, &admin)?;
        if cap < 0 {
            return Err(Error::InvalidAmount);
        }

        let cap_opt = if cap == 0 { None } else { Some(cap) };
        env.storage()
            .persistent()
            .set(&PaymentsDataKey::AssetCap(asset.clone()), &cap_opt);

        env.events().publish(
            (symbol_short!("CAP_SET"), asset.clone()),
            CapSetEvent { asset, cap },
        );
        Ok(OK)
    }

    /// The effective per-transaction cap for `asset` (0 = unlimited).
    pub fn get_effective_cap(env: Env, asset: Symbol) -> i128 {
        effective_cap(&env, &asset).unwrap_or(0)
    }

    // =======================================================================
    // Deposits / withdrawals / transfers
    // =======================================================================

    /// Deposit `amount` of `asset` from `from` into the contract.
    ///
    /// Performs a real token transfer with `from.require_auth()` provided by
    /// the transfer itself. Credits the internal balance.
    pub fn deposit(env: Env, from: Address, asset: Symbol, amount: i128) -> Result<Symbol, Error> {
        from.require_auth();

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        let token = TokenClient::new(&env, &token_address(&env, &asset)?);

        let new_balance = get_balance(&env, &from, &asset)
            .checked_add(amount)
            .ok_or(Error::ArithmeticOverflow)?;

        // Effects first, then the token transfer (both succeed or the whole
        // invocation reverts).
        put_balance(&env, &from, &asset, new_balance);
        token.transfer(&from, &env.current_contract_address(), &amount);

        emit_deposit(&env, &asset, &from, amount, new_balance);
        Ok(OK)
    }

    /// Withdraw `amount` of `asset` to `to` from the caller's internal balance.
    pub fn withdraw(env: Env, to: Address, asset: Symbol, amount: i128) -> Result<Symbol, Error> {
        // The caller is the balance owner; the token transfer authenticates them.
        let caller = to.clone();
        caller.require_auth();

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        check_cap(&env, &asset, amount)?;

        let token = TokenClient::new(&env, &token_address(&env, &asset)?);

        let balance = get_balance(&env, &to, &asset);
        let new_balance = checked_debit(balance, amount)?;

        // Effects first, then the outbound transfer.
        put_balance(&env, &to, &asset, new_balance);
        token.transfer(&env.current_contract_address(), &to, &amount);

        emit_withdraw(&env, &asset, &to, amount, new_balance);
        Ok(OK)
    }

    /// Transfer `amount` of `asset` between internal accounts.
    ///
    /// The tokens stay in the contract; only the internal ledger moves, so no
    /// token transfer occurs — but `from` must still authorize the move.
    pub fn transfer(
        env: Env,
        from: Address,
        to: Address,
        asset: Symbol,
        amount: i128,
    ) -> Result<Symbol, Error> {
        from.require_auth();

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        if from == to {
            return Err(Error::InvalidAmount);
        }
        check_cap(&env, &asset, amount)?;

        let from_balance = get_balance(&env, &from, &asset);
        let new_from = checked_debit(from_balance, amount)?;
        let to_balance = get_balance(&env, &to, &asset);
        let new_to = to_balance
            .checked_add(amount)
            .ok_or(Error::ArithmeticOverflow)?;

        put_balance(&env, &from, &asset, new_from);
        put_balance(&env, &to, &asset, new_to);

        emit_transfer(&env, &asset, &from, &to, amount, new_from, new_to);
        Ok(OK)
    }

    /// Internal balance of `owner` in `asset`.
    pub fn balance_of(env: Env, owner: Address, asset: Symbol) -> i128 {
        get_balance(&env, &owner, &asset)
    }

    // =======================================================================
    // Escrow
    // =======================================================================

    /// Create a two-party escrow: locks `amount` of `asset` from the
    /// depositor's internal balance until released or refunded.
    pub fn create_escrow(
        env: Env,
        depositor: Address,
        receiver: Address,
        asset: Symbol,
        amount: i128,
    ) -> Result<u64, Error> {
        depositor.require_auth();

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        if depositor == receiver {
            return Err(Error::InvalidAmount);
        }
        check_cap(&env, &asset, amount)?;

        // Debit the depositor's internal balance.
        let balance = get_balance(&env, &depositor, &asset);
        let new_balance = checked_debit(balance, amount)?;
        put_balance(&env, &depositor, &asset, new_balance);

        // Allocate the escrow id.
        let escrow_id: u64 = env
            .storage()
            .persistent()
            .get(&PaymentsDataKey::EscrowIdCounter)
            .unwrap_or(0u64)
            .checked_add(1)
            .ok_or(Error::ArithmeticOverflow)?;
        env.storage()
            .persistent()
            .set(&PaymentsDataKey::EscrowIdCounter, &escrow_id);
        env.storage()
            .persistent()
            .set(&PaymentsDataKey::EscrowCount, &escrow_id);

        let record = EscrowRecord {
            escrow_id,
            asset: asset.clone(),
            depositor: depositor.clone(),
            receiver: receiver.clone(),
            amount,
            status: EscrowStatus::Active,
            created_at: env.ledger().timestamp(),
            settled_at: 0,
        };
        env.storage()
            .persistent()
            .set(&PaymentsDataKey::Escrow(escrow_id), &record);

        env.events().publish(
            (symbol_short!("ESC_NEW"), escrow_id),
            EscrowCreatedEvent {
                escrow_id,
                asset,
                depositor,
                receiver,
                amount,
            },
        );
        Ok(escrow_id)
    }

    /// Release an active escrow to its receiver (either party may call).
    pub fn release_escrow(env: Env, escrow_id: u64, caller: Address) -> Result<Symbol, Error> {
        caller.require_auth();
        Self::settle_escrow(&env, escrow_id, &caller, true)
    }

    /// Refund an active escrow to its depositor (either party may call).
    pub fn refund_escrow(env: Env, escrow_id: u64, caller: Address) -> Result<Symbol, Error> {
        caller.require_auth();
        Self::settle_escrow(&env, escrow_id, &caller, false)
    }

    fn settle_escrow(
        env: &Env,
        escrow_id: u64,
        caller: &Address,
        release: bool,
    ) -> Result<Symbol, Error> {
        let key = PaymentsDataKey::Escrow(escrow_id);
        let mut record: EscrowRecord = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::EscrowNotFound)?;

        if record.status != EscrowStatus::Active {
            return Err(Error::EscrowNotActive);
        }
        if *caller != record.depositor && *caller != record.receiver {
            return Err(Error::NotEscrowParty);
        }

        let (to, outcome) = if release {
            (record.receiver.clone(), symbol_short!("release"))
        } else {
            (record.depositor.clone(), symbol_short!("refund"))
        };

        // Effects first: mark settled, then move funds.
        record.status = if release {
            EscrowStatus::Released
        } else {
            EscrowStatus::Refunded
        };
        record.settled_at = env.ledger().timestamp();
        env.storage().persistent().set(&key, &record);

        let token = TokenClient::new(env, &token_address(env, &record.asset)?);
        token.transfer(&env.current_contract_address(), &to, &record.amount);

        env.events().publish(
            (symbol_short!("ESC_SET"), escrow_id),
            EscrowSettledEvent {
                escrow_id,
                outcome,
                to: to.clone(),
                amount: record.amount,
            },
        );
        Ok(OK)
    }

    /// Get an escrow record by id.
    pub fn get_escrow(env: Env, escrow_id: u64) -> Result<EscrowRecord, Error> {
        env.storage()
            .persistent()
            .get(&PaymentsDataKey::Escrow(escrow_id))
            .ok_or(Error::EscrowNotFound)
    }

    /// List the most recent escrows, newest last, bounded by
    /// [`records::MAX_ESCROWS_RETURN`].
    pub fn list_escrows(env: Env) -> Vec<EscrowRecord> {
        let total: u64 = env
            .storage()
            .persistent()
            .get(&PaymentsDataKey::EscrowCount)
            .unwrap_or(0);
        let mut out = Vec::new(&env);
        let start = total.saturating_sub(MAX_ESCROWS_RETURN as u64);
        for id in (start + 1)..=total {
            if let Some(record) = env
                .storage()
                .persistent()
                .get::<PaymentsDataKey, EscrowRecord>(&PaymentsDataKey::Escrow(id))
            {
                out.push_back(record);
            }
        }
        out
    }
}
