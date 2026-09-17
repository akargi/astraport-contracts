//! Storage keys and shared records for the AstraPort Payments contract.
//!
//! Soroban SDK v21 does not support `Option<Address>` inside `#[contracttype]`
//! structs, so "optional" values (like an unset escrow counter) are represented
//! with defaults on read, mirroring the conventions used by the fee contract.

use soroban_sdk::{contracterror, contracttype, Address, Symbol, Vec};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of escrow records returned by `list_escrows`.
pub const MAX_ESCROWS_RETURN: u32 = 200;

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

/// Top-level storage keys used by the payments contract.
#[contracttype]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaymentsDataKey {
    /// The contract administrator address.
    Admin,
    /// Registered token contract for a `Symbol` asset: asset -> token address.
    TokenMap(Symbol),
    /// Internal account balance: (owner, asset).
    Balance(Address, Symbol),
    /// Global per-transaction spending cap (base units).
    GlobalCap,
    /// Per-asset per-transaction spending cap: asset -> amount.
    AssetCap(Symbol),
    /// Monotonically increasing escrow id counter.
    EscrowIdCounter,
    /// Escrow record by id: id -> EscrowRecord.
    Escrow(u64),
    /// Total number of escrows ever created (for bounded enumeration).
    EscrowCount,
}

// ---------------------------------------------------------------------------
// Escrow records
// ---------------------------------------------------------------------------

/// Lifecycle status of an escrow.
#[contracttype]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscrowStatus {
    /// Funds are locked and waiting for release or refund.
    Active,
    /// Funds were released to the receiver.
    Released,
    /// Funds were refunded to the depositor.
    Refunded,
}

/// A two-party escrow holding real tokens until released or refunded.
#[contracttype]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EscrowRecord {
    /// Unique escrow id.
    pub escrow_id: u64,
    /// The asset symbol this escrow is denominated in.
    pub asset: Symbol,
    /// The account that funded the escrow.
    pub depositor: Address,
    /// The account entitled to receive the funds on release.
    pub receiver: Address,
    /// Amount locked, in base units.
    pub amount: i128,
    /// Current lifecycle status.
    pub status: EscrowStatus,
    /// Ledger timestamp at creation.
    pub created_at: u64,
    /// Ledger timestamp at release/refund (0 while active).
    pub settled_at: u64,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Event emitted on a successful deposit.
#[contracttype]
#[derive(Debug, Clone)]
pub struct DepositEvent {
    pub asset: Symbol,
    pub from: Address,
    pub amount: i128,
    pub new_balance: i128,
}

/// Event emitted on a successful withdrawal.
#[contracttype]
#[derive(Debug, Clone)]
pub struct WithdrawEvent {
    pub asset: Symbol,
    pub to: Address,
    pub amount: i128,
    pub new_balance: i128,
}

/// Event emitted on an internal transfer between two accounts.
#[contracttype]
#[derive(Debug, Clone)]
pub struct TransferEvent {
    pub asset: Symbol,
    pub from: Address,
    pub to: Address,
    pub amount: i128,
    pub from_balance: i128,
    pub to_balance: i128,
}

/// Event emitted when escrow funds are locked.
#[contracttype]
#[derive(Debug, Clone)]
pub struct EscrowCreatedEvent {
    pub escrow_id: u64,
    pub asset: Symbol,
    pub depositor: Address,
    pub receiver: Address,
    pub amount: i128,
}

/// Event emitted when escrow funds are released or refunded.
#[contracttype]
#[derive(Debug, Clone)]
pub struct EscrowSettledEvent {
    pub escrow_id: u64,
    pub outcome: Symbol, // "release" | "refund"
    pub to: Address,
    pub amount: i128,
}

/// Event emitted when the admin role changes.
#[contracttype]
#[derive(Debug, Clone)]
pub struct AdminSetEvent {
    pub old_admin: Address,
    pub new_admin: Address,
}

/// Event emitted when a spending cap changes (asset is the empty symbol for the global cap).
#[contracttype]
#[derive(Debug, Clone)]
pub struct CapSetEvent {
    pub asset: Symbol,
    pub cap: i128, // 0 = disabled
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors returned by the payments contract.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// Contract has already been initialized.
    AlreadyInitialized = 1,
    /// Contract has not been initialized yet.
    NotInitialized = 2,
    /// Caller is not the admin.
    Unauthorized = 3,
    /// Amount must be positive.
    InvalidAmount = 4,
    /// The asset symbol has no registered token contract.
    TokenNotRegistered = 5,
    /// The caller's internal balance is insufficient.
    InsufficientBalance = 6,
    /// Arithmetic overflow or underflow.
    ArithmeticOverflow = 7,
    /// The requested transfer exceeds the applicable spending cap.
    CapExceeded = 8,
    /// Escrow id not found.
    EscrowNotFound = 9,
    /// Escrow is not in the `Active` state.
    EscrowNotActive = 10,
    /// Only the escrow depositor or receiver may perform this action.
    NotEscrowParty = 11,
    /// The token list is full or a limit was exceeded.
    LimitExceeded = 12,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Sentinel asset symbol used for global (non-asset-specific) configuration.
///
/// Uses the `GLOB` short symbol ("GLOB") since Soroban short symbols only
/// allow alphanumeric characters and underscores.
pub fn global_asset_symbol() -> Symbol {
    use soroban_sdk::symbol_short;
    symbol_short!("GLOB")
}

/// Sum the escrow amounts in `records` (used by tests and reporting).
pub fn sum_escrow_amounts(records: &Vec<EscrowRecord>) -> i128 {
    let mut total: i128 = 0;
    for r in records.iter() {
        total = total.saturating_add(r.amount);
    }
    total
}
