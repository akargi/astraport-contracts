# AstraPort Contracts — Module Feature Issues

A curated backlog of 20 GitHub-ready issues, one per major workstream, derived from a full
review of the workspace (`contracts/*`, `tests/`, `.github/workflows/rust-checks.yml`, `docs/`).

Each issue is self-contained and ready to paste into `gh issue create` or the GitHub UI.
Suggested labels are listed per issue; a consistent label taxonomy would be:

- `module:<crate>` — e.g. `module:staking`, `module:prediction`
- `type:feature` | `type:fix` | `type:integration` | `type:hardening`
- `priority:P0` (blocks build/correctness) · `P1` (high value) · `P2` (nice to have)

**Table of contents**

| # | Issue | Module | Priority |
|---|-------|--------|----------|
| 1 | Implement the missing Payments contract (empty crate breaks the workspace build) | payments | P0 |
| 2 | Execute real token swaps in the multi-asset rebalancer | rebalancing | P0 |
| 3 | Keeper-executed scheduled rebalancing with due windows and incentives | rebalancing | P1 |
| 4 | Token escrow integration for staking (real deposits, not internal ledgers) | staking | P0 |
| 5 | Real collateral token integration for prediction markets | prediction | P0 |
| 6 | Fix hardcoded `num_outcomes = 3` in settlement position tracking | prediction | P0 |
| 7 | Scalable, paginated market registry for prediction markets | prediction | P1 |
| 8 | Wire the trade engine to the fee contract | trade | P1 |
| 9 | Price-level indexing for O(log n) order book matching | trade, prediction | P1 |
| 10 | True TWAP implementation with a configurable window | pricefeed | P1 |
| 11 | Bounded retention for price history | pricefeed | P1 |
| 12 | Live valuation via the pricefeed contract + scheduled snapshots | valuation | P1 |
| 13 | Proposal action execution dispatch for governance | governance | P1 |
| 14 | Token-backed voting power (locked/staked balances) | governance | P2 |
| 15 | Perform real on-chain upgrades in `execute_upgrade` | versioning | P1 |
| 16 | Cross-contract subscriber dispatch for the events contract | events | P1 |
| 17 | Automatic circuit-breaker tripping from the pricefeed | emergency | P1 |
| 18 | On-chain ed25519 verification entrypoint for signed audit exports | audit | P2 |
| 19 | Real reward payouts for gamification (token-backed reward pool) | gamification | P2 |
| 20 | Replace placeholder integration tests and harden CI | cross-cutting | P0 |

---

## 1. Implement the missing Payments contract (empty crate breaks the workspace build)

**Module:** `payments` · **Labels:** `module:payments`, `type:feature`, `priority:P0`

### Background

`Cargo.toml` lists `contracts/payments` as a workspace member, but the crate directory is
empty — `contracts/payments/src/` contains no files and there is no `Cargo.toml`. Every
workspace-wide command (`cargo build`, `cargo test`, `cargo clippy`) fails or skips because
the member does not resolve. No other file in the repository references "payments", so the
module is entirely undefined.

### Direction

Implement a minimal-but-complete payments/settlement crate consistent with the suite's
existing conventions (`#![no_std]`, `#[contracterror]` enums, `symbol_short!` storage keys
or a `DataKey` enum, one-time `initialize(admin)`, `require_auth()` on user-facing entry
points, and Soroban events for state changes).

Recommended scope for v1:

- `deposit(asset, from, amount)` / `withdraw(asset, to, amount)` using the Soroban token
  client (`soroban_sdk::token::Client`), with balances tracked per (owner, asset).
- `transfer(from, to, asset, amount)` between internal accounts.
- An `admin` role plus `transfer_admin` for rotation.
- A per-asset and global spending cap, mirroring the style of the emergency contract's
  `max_trade_amount` guard.
- Events: `DEPOSIT`, `WITHDRAW`, `TRANSFER`, `ADMIN_SET`.

Decide and document whether "payments" is the protocol's escrow/settlement layer for other
crates (trade, prediction, staking) — if so, expose `escrow` / `release` / `refund`
entry points designed for cross-contract calls, since issues #5 and #8 will consume it.

### Acceptance criteria

- [ ] `contracts/payments/Cargo.toml` exists with `astraport-payments` package name and the
      workspace `soroban-sdk = =21.5.0` dependency.
- [ ] `contracts/payments/src/lib.rs` compiles under `#![no_std]` and exposes the contract
      struct via `#[contract]` / `#[contractimpl]`.
- [ ] `initialize` is one-time; subsequent calls return a typed error, not a panic.
- [ ] Deposit/withdraw/transfer perform real token transfers via `token::Client` with
      `require_auth()` on the moving party.
- [ ] All arithmetic uses `checked_add` / `checked_sub` with typed error variants.
- [ ] Deposit, withdraw, transfer, and admin events are emitted.
- [ ] Unit tests cover: happy paths, insufficient balance, unauthorized caller,
      double-initialize, and overflow attempts (`cargo test -p astraport-payments` green).
- [ ] `cargo build --workspace` and `cargo test --workspace` succeed with the crate included.

### Out of scope

Multi-token fee routing (owned by the fee contract) and recurring subscriptions.

---

## 2. Execute real token swaps in the multi-asset rebalancer

**Module:** `rebalancing` · **Labels:** `module:rebalancing`, `type:fix`, `type:integration`, `priority:P0`

### Background

`MultiAssetRebalancer::rebalance` (`contracts/rebalancing/src/multi_asset_rebalancer.rs`,
line 70) still contains the original TODO:

```rust
// TODO: Execute the trades using Soroban token interface
```

Worse, the surrounding logic runs entirely on mocks: portfolio value is hard-coded to
`1_000_000_000_000`, prices are hard-coded (`sell_price = 50000`, `buy_price = 1`), trades
are matched 1:1 between the sell list and buy list regardless of notional, and fees/
slippage are derived from a magic `/ 1_000_000`. `simulate_rebalance` returns these mock
numbers to users as if they were real estimates. The trade engine (`astraport-trade`) and
price feed (`astraport-pricefeed`) already exist and are the natural execution and pricing
backends.

### Direction

1. Accept real execution venues: add a strategy/venue parameter that either (a) calls the
   trade engine's order placement/matching via its generated client, or (b) executes
   direct token swaps against a configured AMM/token pair. Start with (a) — it already has
   slippage protection and atomic batch semantics.
2. Source prices from `astraport-pricefeed` (`get_price` / `get_price_with_method`) instead
   of hard-coded constants; fail with a typed error when a price is stale or missing.
3. Size trades from actual holdings: replace the mock portfolio value with the portfolio's
   real asset balances (asset contract or holdings passed in), converting `drift_bps` into
   notional amounts via the price feed.
4. Match sells to buys by notional value, not by list position (greedy largest-first is
   sufficient for v1).
5. Keep `simulate_rebalance` mock-free: it should run the same pricing/sizing path but
   stop before execution.
6. Emit a `REBAL_EXECUTED` event with per-trade results and audit-log the run via the
   existing `AuditLogger` sink used elsewhere in the crate.

### Acceptance criteria

- [ ] The `TODO` comment and both hard-coded price constants and the mock portfolio value
      are removed from `multi_asset_rebalancer.rs`.
- [ ] Trades are executed through `token::Client` transfers or the trade-engine client;
      no state-changing path depends on mock numbers.
- [ ] Stale/missing price data produces a typed `RebalanceError` variant, not a bad trade.
- [ ] Sell/buy matching pairs by notional value; a test proves unequal-side portfolios
      (e.g. 3 sells, 1 buy) settle correctly.
- [ ] `simulate_rebalance` and `rebalance` share the sizing code path; simulation results
      equal execution results when prices do not move.
- [ ] Unit tests cover: normal rebalance, missing price, oversized drift, and zero
      adjustments. `cargo test -p astraport-rebalancing` stays green.

---

## 3. Keeper-executed scheduled rebalancing with due windows and incentives

**Module:** `rebalancing` · **Labels:** `module:rebalancing`, `type:feature`, `priority:P1`

### Background

`RebalancingSchedule { interval, next_execution, last_execution }` and
`RebalanceInterval::{Hourly, Daily, Weekly, Monthly}` already exist, and `set_schedule`
persists them. Soroban has no cron, so schedules currently do nothing until someone calls
`rebalance` manually — there is no "execute if due" entry point, no grace window, and no
incentive for a third party to keep portfolios on schedule.

### Direction

Add a keeper pattern:

- `execute_rebalance_if_due(owner, portfolio_id, strategy, caller)` — callable by *anyone*
  when `now >= next_execution` and `now <= next_execution + grace_window`. RBAC checks
  (`CAN_EXECUTE_REBALANCE`) apply to early/manual calls but the keeper path bypasses them
  *only* when the schedule is genuinely due.
- `set_keeper_config(portfolio_id, grace_window_secs, keeper_reward_bps)` — admin/owner
  gated. The reward is paid from the fee/valuation layer or accrued as an internal credit
  (decide with issue #8's fee work; v1 can record the reward as an internal balance claimable
  via `claim_keeper_reward`).
- On execution: recompute drift with the drift engine, execute via issue #2's real
  execution path, bump `last_execution`/`next_execution`, and emit `SCHEDULE_EXECUTED`
  with keeper + portfolio IDs.
- Guard rails: minimum interval between executions, max executions per portfolio per
  window, and a `cancel_schedule` that clears `next_execution`.

### Acceptance criteria

- [ ] A portfolio with an overdue schedule can be rebalanced by an arbitrary account;
      the same call before `next_execution` fails with a typed `ScheduleNotDue` error.
- [ ] Calls after `next_execution + grace_window` fail (prevents stale keeper runs); the
      owner can always execute manually regardless of timing.
- [ ] Keeper reward is credited exactly once per execution and is claimable.
- [ ] `next_execution` advances by the correct interval for all four `RebalanceInterval`
      variants (tested with timestamp jumps).
- [ ] RBAC: manual (early) execution still requires `CAN_EXECUTE_REBALANCE`; keeper
      execution of a due schedule does not.
- [ ] Events emitted for schedule execution, cancellation, and config changes.
- [ ] Tests cover: due/not-yet-due/expired-window, reward claim, interval math, and RBAC
      bypass rules.

---

## 4. Token escrow integration for staking (real deposits, not internal ledgers)

**Module:** `staking` · **Labels:** `module:staking`, `type:integration`, `priority:P0`

### Background

Staking balances (`StakeDataKey::Balance`), yield reserves, and claims are all internal
i128 ledgers. `stake` / `unstake` / `claim_yield` move bookkeeping numbers but never touch
a token contract, so the contract holds no real collateral and a staker's "unstaked"
amount is not actually transferred anywhere. Every other money-touching module (payments,
prediction) has or will have the same need, so staking should establish the pattern.

### Direction

- Add an optional `set_staking_token(admin, asset, token_id)` registration map so each
  `Symbol` asset maps to a Soroban token contract address (mirrors issue #5 for prediction).
- `stake`: `token::Client::new(&env, token_id).transfer(&staker, &contract, &amount)` before
  crediting the internal balance; the `require_auth` of the transfer authenticates the staker.
- `unstake` and `claim_yield*`: transfer out from the contract to the staker, credit/debit
  internal ledgers in the same order as the current check-effects-then-transfer flow, and
  keep the existing lock/emergency-penalty math untouched.
- Support a per-asset `yield_reserve` funded by `fund_yield_reserve(asset, token, amount)`
  with a real transfer in, so distribution claims are backed by held tokens.
- Introduce a `paused` flag hook (or call into `astraport-emergency`) so transfers can be
  halted in an incident — coordinate with issue #17.
- Migration: if the internal ledger is non-zero at registration time, refuse to register
  (or provide an explicit admin sweep) so legacy mock balances cannot be redeemed twice.

### Acceptance criteria

- [ ] Each staked `Symbol` asset resolves to a registered token contract; unregistered
      assets are rejected with a typed error.
- [ ] `stake` performs a real `transfer` in; a mocked token client test proves the
      contract's token balance increases by exactly `amount`.
- [ ] `unstake`, `claim_yield`, and `claim_yield_partial` transfer out the exact claimed
      amount and fail closed if the contract's real balance is insufficient.
- [ ] Lock schedules, emergency penalties, and cooldown behavior are unchanged (existing
      tests in `contracts/staking/src/tests.rs` pass without modification).
- [ ] Reentrancy-style safety: internal state is finalized before the outbound transfer.
- [ ] Tests use `soroban-sdk`'s `token::StellarAssetClient`/test token with
      `mock_all_auths`, covering stake, unstake, claim, reserve funding, and
      insufficient-contract-balance failures.

---

## 5. Real collateral token integration for prediction markets

**Module:** `prediction` · **Labels:** `module:prediction`, `type:integration`, `priority:P0`

### Background

Every market is hard-wired to `collateral_token: symbol_short!("USDC")`
(`contracts/prediction/src/lib.rs`, line 117). Collateral balances, LP deposits, swaps,
fees, and redemptions are all internal ledger entries; no USDC (or any token) ever moves.
This makes the market's economics unenforceable — a user can buy outcome tokens without
ever depositing collateral.

### Direction

- Add `set_collateral_token(admin, token: Address)` once at (or right after)
  initialization; `Market.collateral_token` becomes derived from that global setting
  (or per-market override passed to `create_market`).
- `deposit_collateral(user, amount)`: real `token::Client` transfer in, credited to
  `CollateralBalance(user)`; `withdraw_collateral(user, amount)` transfers out.
- Route every collateral-moving flow through deposits: `create_liquidity_pool` /
  `add_liquidity` pull from the user's deposited balance (or transfer directly with the
  user's auth), `remove_liquidity`, `redeem_tokens` (winning redemptions), LP fee
  distribution, and refund paths for `cancel_market` all pay out with real transfers.
- On `cancel_market`, refund outstanding collateral to LPs/participants.
- Keep CPMM math untouched — only the settlement edges gain token movements.
- Emit `COLLATERAL_DEPOSITED` / `COLLATERAL_WITHDRAWN` / `PAYOUT` events for indexers.

### Acceptance criteria

- [ ] No market can be created until a collateral token is registered; the hard-coded
      `symbol_short!("USDC")` is gone.
- [ ] Buying outcome tokens requires available deposited collateral of exactly that
      amount; insufficient balance fails with `InsufficientBalance` before pool math runs.
- [ ] LPs who `remove_liquidity` receive real collateral; winners of `redeem_tokens`
      receive real collateral; a mocked-token test asserts exact balances after each flow.
- [ ] `cancel_market` refunds participants and zeroes their internal balances.
- [ ] The CPMM invariant checks (`check_invariant`, `verify_pool_invariant`) still pass in
      all existing tests.
- [ ] Tests cover: deposit → buy → resolve → redeem happy path; withdraw-more-than-deposit
      rejection; LP add/remove round trip; fee distribution in real tokens.

---

## 6. Fix hardcoded `num_outcomes = 3` in settlement position tracking

**Module:** `prediction` · **Labels:** `module:prediction`, `type:fix`, `priority:P0`

### Background

`settlement::record_trade` (`contracts/prediction/src/settlement.rs`, line 26) initializes
new user positions with a literal:

```rust
let num_outcomes = 3u32; // Will be updated from market
```

Every binary market (2 outcomes) allocates a 3-slot position, and any market with more
than 3 outcomes corrupts position bookkeeping for trades on outcomes ≥ 3 — the vector is
grown lazily afterward, but entry prices and amounts start life mis-sized, and any code
that assumes `len() == outcomes.len()` (PnL computation, settlement) can misindex. The
comment admits it was never finished.

### Direction

- `record_trade` already receives `market_id`; load the `Market` from storage and use
  `market.outcomes.len()` for sizing. Return `MarketNotFound` instead of panicking if
  absent.
- Add a `normalize_position(&mut Position, expected_len)` helper that pads legacy
  3-sized vectors with zeros, so existing stored positions remain readable.
- Audit every `Position` consumer (`settle_position`, `redeem_winning_tokens`, PnL
  getters) for index assumptions and route them through the market's outcome count.
- Add a defensive invariant check in tests: `position.outcome_amounts.len() ==
  market.outcomes.len()` after any trade.

### Acceptance criteria

- [ ] The literal `3u32` and the "Will be updated from market" comment are removed.
- [ ] Binary market trades create 2-slot positions; 10-outcome market trades create
      10-slot positions (unit tested for both).
- [ ] Settlement and redemption compute correct PnL for outcome indices 0 and 1 on binary
      markets and 0–9 on a max-size market.
- [ ] Existing tests pass unchanged; new tests cover the previously broken ≥3-outcome
      indices.
- [ ] A regression test proves positions persist correctly across multiple trades by the
      same user in multi-outcome markets.

---

## 7. Scalable, paginated market registry for prediction markets

**Module:** `prediction` · **Labels:** `module:prediction`, `type:hardening`, `priority:P1`

### Background

`PredictionDataKey::MarketList` is a single persistent `Vec<u64>` that every
`create_market` appends to, and `get_all_market_ids` returns the whole thing. The module
declares `MAX_ACTIVE_MARKETS = 10_000` ("design target: 1M+ but capped at Vec limits"),
but a single Soroban ledger entry holding 10k IDs already brushes against ledger-entry
size limits, and every append rewrites the whole vector — O(n) gas growth per market.
`MarketsByCategory` index vectors have the same unbounded-growth problem.

### Direction

- Replace the monolithic list with chunked pages: `MarketPage(u32) -> Vec<u64>` holding a
  fixed page size (e.g. 128), plus a `MarketCount` counter. `create_market` writes only
  the affected page.
- Add paginated read entry points: `get_market_ids_page(page: u32, limit: u32)` and
  `get_market_count()`, keeping `get_all_market_ids` as a bounded convenience (first N)
  or deprecating it.
- Apply the same paging to `MarketsByCategory` with
  `get_markets_by_category_page(category, page, limit)`.
- Add an optional status filter entry point (`get_market_ids_by_status`) so off-chain
  services stop scanning resolved markets.
- Keep `MAX_ACTIVE_MARKETS` enforced via the counter, and document the chunk size choice
  in `docs/ARCHITECTURE.md`.

### Acceptance criteria

- [ ] Creating market N touches O(page-size) storage, not O(N); verified by a test that
      creates ~500 markets and asserts a bounded read set (or via snapshot size).
- [ ] `get_market_ids_page` returns stable, ordered pages with correct boundary behavior
      (empty page, partial final page, out-of-range page).
- [ ] Category indexes paginate identically and stay consistent with the global list.
- [ ] `get_all_market_ids` (if retained) is documented as bounded and never exceeds one
      page's worth of data in tests.
- [ ] Migration note: freshly deployed contracts only; no in-place storage migration
      required (state clearly in the PR description if a migration path is added).

---

## 8. Wire the trade engine to the fee contract

**Module:** `trade` · **Labels:** `module:trade`, `type:integration`, `priority:P1`

### Background

The trade engine tracks `total_fees` on `TradeLeg` and computes fee amounts internally,
while the separate fee contract (`astraport-fee`) already implements flat/percentage/
tiered structures, portfolio-fee assignment, waivers, discounts, caps, revenue
distribution, and reporting. The two are not connected: trades settle without consulting
configured fee structures, and collected fees never reach revenue recipients.

### Direction

- Add `set_fee_sink(admin, fee_contract: Address)`; when unset, keep current behavior
  (backwards compatible).
- On each fill (or batch), compute the fee via the fee contract's
  `calculate_portfolio_fee` / `estimate_fee` client call using the trader's portfolio
  mapping, apply waivers/discounts there, and record collection via `collect_fee`.
- Replace ad-hoc `total_fees` accumulation with the fee contract's figures so
  `get_fee_history` and revenue distribution see real flow.
- Emit a `FEE_COLLECTED` event per fill (or per batch) with pair, trader, amount, and
  fee id; forward failures as a typed `TradeError::FeeCollectionFailed` that does *not*
  roll back the fill itself if the policy is "best effort" — make the policy explicit
  and configurable per pair (`strict_fees: bool` in `SlippageConfig`-style pair config).
- Document the integration in `docs/INTEGRATION_GUIDE.md`.

### Acceptance criteria

- [ ] With a fee sink configured, every fill produces a fee record in the fee contract
      with the correct structure-derived amount (percentage, flat, and tiered tested).
- [ ] Fee waivers and discounts configured in the fee contract are honored by trades.
- [ ] Revenue distribution (`distribute_revenue_amount`) can be driven from collected
      trade fees in an integration test.
- [ ] Without a fee sink, behavior is byte-identical to today (existing tests pass).
- [ ] `strict_fees = true` causes the fill to fail if fee collection fails; the failure
      is a typed error and leaves no partial state (order book untouched).
- [ ] Documentation updated with a sequence description of the trade→fee call.

---

## 9. Price-level indexing for O(log n) order book matching

**Module:** `trade`, `prediction` · **Labels:** `module:trade`, `module:prediction`, `type:hardening`, `priority:P1`

### Background

Both order books (`contracts/trade/src/orderbook.rs` and
`contracts/prediction/src/orderbook.rs`) store orders in flat vectors and match by
linear scans (`while remaining_amount > 0 && idx < book.asks.len()` walking every entry,
or filtering by predicate over the whole book). Cancels similarly scan for the order id.
With realistic order counts this becomes the dominant gas cost and, at ledger limits, a
DoS vector — a cheap fill order pays for walking thousands of stale orders.

### Direction

- Maintain a sorted price-level index per side: `Vec<(price, level_start_idx)>` kept in
  ascending (asks) / descending (bids) order, or switch storage to a
  `Map<price, Vec<order_id>>` levels map plus a sorted key cache.
- Matching walks levels best-first and stops as soon as `remaining == 0` (already true in
  trade's inner loop, but level indexing makes the *outer* search logarithmic).
- Cancellation removes by order id via an `order_index: Map<u64, u32>` position map or
  lazy tombstoning (mark cancelled, skip during match, compact opportunistically).
- Keep `get_order_book` snapshot semantics identical for consumers.
- Apply the same structure to both crates (extract a shared doc note; a shared crate is
  welcome but optional — avoid breaking the independent `TradeDataKey`/`PredictionDataKey`
  layouts).

### Acceptance criteria

- [ ] Placing, matching, and cancelling remain functionally identical: every existing
      test in both crates passes without assertion changes.
- [ ] A benchmark test (e.g. 500 resting orders, 100 incoming) completes and documents
      measured iteration counts before/after in the PR description.
- [ ] Cancellation of an arbitrary order id is no longer O(n) over the whole book (or is
      tombstoned and proven correct by re-fill tests).
- [ ] Snapshot format (`OrderBookSnapshot`) is unchanged; clients see identical output.
- [ ] Invariant test: after any operation sequence, price-time priority of fills matches
      a reference linear-scan implementation on randomized inputs (property test with a
      fixed seed).

---

## 10. True TWAP implementation with a configurable window

**Module:** `pricefeed` · **Labels:** `module:pricefeed`, `type:fix`, `priority:P1`

### Background

`AggregateEngine::aggregate_twap` (`contracts/pricefeed/src/aggregation.rs`, lines
101–160) is not a time-weighted average price. It weights each observation by
`1_000_000 − age_seconds` — a recency heuristic with an arbitrary 1e6-second horizon that
silently degrades to zero weight for anything older, ignores how long each price was
*valid* for, and has no configurable window. For rebalancing and valuation consumers this
mislabels a recency-weighted spot average as TWAP.

### Direction

- Implement interval TWAP over the price history the contract already records
  (`record_price_history`): TWAP = Σ(price_i × duration_i) / Σ(duration_i), where
  duration_i is how long price_i was the latest known price inside the window.
- Add `window_secs` to `PriceValidationConfig` (or a new `AggregationConfig`) with sane
  bounds (e.g. 60 s … 7 days) and per-request override via
  `get_price_with_method(asset, method, window_secs)`.
- Fall back explicitly: if history does not cover the window, aggregate over the available
  span and set `status` to a new `PriceStatus::PartialWindow` (or document the reuse of an
  existing status) rather than silently returning a short-window average.
- Keep the existing recency weighting available under a new
  `AggregationMethod::RecencyWeighted` if any consumer wants it; default `TWAP` to the
  correct implementation.
- Unit-test with synthetic timelines: constant price, step change, and linear ramp — the
  ramp's TWAP must equal the analytic average.

### Acceptance criteria

- [ ] TWAP over a linear ramp from P0 to P1 over the window equals the midpoint within
      fixed-point tolerance (test asserted).
- [ ] Duration weighting is proven: a price that held 90% of the window dominates the
      result versus an equal-count averaging.
- [ ] Window is configurable and validated (zero/negative rejected with a typed error).
- [ ] Partial-window queries are flagged in `status`, not silently shortened.
- [ ] Existing `Latest`/`Median`/`WeightedAverage` behavior and tests are unchanged.
- [ ] `docs/API_REFERENCE.md` updated to describe the corrected semantics.

---

## 11. Bounded retention for price history

**Module:** `pricefeed` · **Labels:** `module:pricefeed`, `type:hardening`, `priority:P1`

### Background

`validation::record_price_history` appends a `PriceHistoryEntry` to a per-asset vector on
every aggregated price with no cap. Unlike the audit contract (which has
`RetentionPolicy` / `prune_old`) and governance (which caps its audit trail at 5,000
entries), price history grows unbounded — a busy asset accumulates storage cost forever,
entry reads get slower, and eventually the ledger entry hits size limits and breaks the
asset's price feed entirely.

### Direction

- Add a `history_retention: PriceHistoryRetention { max_entries: u32, min_age_secs: u64 }`
  to the validation config, defaulting to something like 1,000 entries.
- Enforce on append: when the cap is hit, drop oldest entries (ring-buffer semantics —
  either rotate the vector head or switch to a `Map<u64 index, entry>` with a head/tail
  cursor; the map approach avoids O(n) rewrites).
- Preserve `get_price_history` and `get_price_history_length` semantics (most recent N,
  ordered newest-last or oldest-first — match current behavior and document it).
- Optional: `prune_history_before(asset, timestamp)` admin entry point for manual
  cleanup, mirroring the audit contract's retention design for consistency.
- Check the same pattern in valuation's history (`ValuationHistoryEntry`) and apply the
  fix there too if it is likewise unbounded.

### Acceptance criteria

- [ ] History length never exceeds the configured cap regardless of submission count
      (test submits 3× cap worth of prices).
- [ ] The surviving entries are exactly the most recent ones, in the documented order.
- [ ] TWAP (issue #10) is tested against the ring buffer so pruning cannot corrupt
      window math at the wrap boundary.
- [ ] Cap is admin-configurable within documented bounds and persisted in the validation
      config struct (backwards-compatible default for existing deployments).
- [ ] Audit note in the PR comparing the approach with the audit contract's
      `RetentionPolicy` for consistency.

---

## 12. Live valuation via the pricefeed contract + scheduled snapshots

**Module:** `valuation` · **Labels:** `module:valuation`, `type:integration`, `priority:P1`

### Background

The valuation contract computes portfolio value, allocation percentages, returns, Sharpe,
Sortino, max drawdown, and TWR — but its inputs are whatever prices callers pass in or
that the asset contract's manually-set prices provide. Nothing connects it to the
pricefeed contract's aggregation, staleness detection, and fallback prices, so "current"
valuations can silently be built from arbitrary or stale numbers.

### Direction

- Add `set_price_source(admin, pricefeed_contract: Address)`; when configured,
  `calculate_portfolio_value` (and any metrics that need prices) fetch per-asset prices
  via the pricefeed client (`get_price` / `get_price_with_method`) instead of caller-
  supplied values.
- Surface freshness: include per-asset `PriceStatus` and the aggregate timestamp in the
  returned valuation record (extend the record struct with an optional `price_meta` map
  so the change is additive for consumers).
- Staleness policy: reject or down-grade valuations containing stale/unavailable prices
  according to a `staleness_policy: Reject | BestEffort` config knob (admin-set);
  `Reject` returns a typed error listing the offending assets.
- Add `snapshot_if_stale(portfolio_id, min_interval_secs)`: an anyone-callable keeper
  entry point that writes a `PortfolioSnapshot` only if the newest snapshot is older than
  the interval — this gives the performance metrics (Sharpe/Sortino/drawdown need a time
  series) a reliable, incentive-compatible data stream without a cron.
- Keep manual price entry working when no price source is configured.

### Acceptance criteria

- [ ] With a price source configured, valuation uses pricefeed prices; a test registers
      two oracles with differing prices and asserts the aggregated value flows through.
- [ ] `staleness_policy = Reject` fails on a stale asset with a typed error naming the
      asset; `BestEffort` succeeds and marks the asset in `price_meta`.
- [ ] `snapshot_if_stale` writes at most one snapshot per interval across concurrent
      calls and is a no-op otherwise (tested with timestamp manipulation).
- [ ] Without a configured source, existing behavior and tests are unchanged.
- [ ] Performance metrics (Sharpe, Sortino, max drawdown, TWR) computed over snapshots
      created by the keeper path match values computed over manually created snapshots.

---

## 13. Proposal action execution dispatch for governance

**Module:** `governance` · **Labels:** `module:governance`, `type:feature`, `priority:P1`

### Background

`execute_proposal` (`contracts/governance/src/lib.rs`, line 1102) transitions a passed,
timelocked proposal to `Executed`, but the docstring states the actual execution "is
handled by the caller or external integration; this records the state change." In
practice nothing executes: `ProposalActionType::{ParameterChange, TreasurySpend,
ProtocolUpgrade, EmergencyAction, RewardDistribution}` carry no executable payload and no
dispatch. Governance today is a voting ledger, not a decision engine.

### Direction

- Extend `Proposal` with a typed `action` payload enum (one variant per
  `ProposalActionType`, e.g. `ParameterChange { key, value }`, `TreasurySpend { request_id }`,
  `ProtocolUpgrade { proposal_id }`, `EmergencyAction { pause: bool, reason }`,
  `RewardDistribution { params }`), stored at submission time and shown in `get_proposal`.
- Implement `execute_proposal` as a dispatcher after the existing timelock checks:
  - `ParameterChange` → apply to `GovernanceConfig` (with an allowlist of
    governable keys to avoid self-dealing on timelock/quorum values).
  - `TreasurySpend` → call the treasury module's execute path for the referenced request
    (multi-sig threshold still applies or is waived by governance passage — make it an
    explicit config).
  - `ProtocolUpgrade` → call the versioning contract's `approve_upgrade`/`execute_upgrade`
    via its client (synergy with issue #15).
  - `EmergencyAction` → invoke the emergency contract's pause/unpause via client.
- Record every dispatched action in the governance audit trail and emit
  `PROPOSAL_EXECUTED` with the action type; on cross-contract failure mark the proposal
  `ExecutionFailed` (status already exists) instead of reverting the whole tally.
- Keep backwards compatibility: proposals submitted without a payload execute as no-ops
  with the old semantics.

### Acceptance criteria

- [ ] A `ParameterChange` proposal changes the governed parameter after
      vote → timelock → execute, with all intermediate state transitions asserted.
- [ ] A `TreasurySpend` proposal triggers the treasury payout path end-to-end in an
      integration test.
- [ ] A `ProtocolUpgrade` proposal reaches the versioning contract (issue #15 test or a
      mocked client).
- [ ] A failing action marks the proposal `ExecutionFailed` and leaves prior state
      intact; it cannot be re-executed after success (`ProposalAlreadyExecuted`).
- [ ] Self-dealing guards: parameter changes to timelock delay / quorum / approval
      threshold via proposal are either blocked or require the emergency timelock path —
      document the chosen policy and test it.
- [ ] Audit trail entries exist for every dispatch.

---

## 14. Token-backed voting power (locked/staked balances)

**Module:** `governance` · **Labels:** `module:governance`, `type:feature`, `priority:P2`

### Background

`deposit_voting_power(amount)` / `withdraw_voting_power(amount)` track voting weight in an
internal ledger, and total supply for quorum math is set manually via
`set_total_supply` (`DEFAULT_QUORUM_BPS` is "4% of total supply"). Voting power is thus
detached from any real asset — anyone can be granted power by the admin, and quorum is
computed against an arbitrary number. Issue #4 gives staking real token escrow, which
this module can build on.

### Direction

- Add `set_power_source(admin, source: PowerSource)` where `PowerSource` is either
  `Internal` (today's behavior, default) or `Token { token: Address }` /
  `Staking { contract: Address }`.
- Under `Token`: `deposit_voting_power` performs a real `token::Client` transfer into the
  governance contract and credits voting weight 1:1; `withdraw_voting_power` transfers
  out (only when no active votes/delegations depend on it).
- Under `Staking`: voting weight mirrors the staking contract's `get_balance` via
  cross-contract read; no transfers happen here, and withdrawing staked funds naturally
  drains power (document the read-at-vote-time semantics to avoid snapshot disputes).
- Quorum base: replace manual `set_total_supply` with `circulating_supply` fetched from
  the token (if the token exposes it via the SAC interface) or keep the manual setting
  but persist it in `GovernanceConfig` with an event and an admin-only guard.
- Keep delegation, `cast_delegated_vote`, and reward claiming unchanged.

### Acceptance criteria

- [ ] `Internal` mode preserves every existing test result.
- [ ] `Token` mode: deposit increases voting power and the contract's real token balance;
      withdraw after the voting period returns funds and restores power; withdrawing
      while delegated or mid-vote fails with a typed error.
- [ ] `Staking` mode: unstaking in the staking contract reduces governance weight on the
      next vote cast (integration test with the staking client).
- [ ] Quorum math uses the configured base and is covered by a test at exactly-threshold
      and just-below-threshold values.
- [ ] Mode switching is admin-only, emits an event, and refuses to switch while any
      proposal is in `Voting` state.

---

## 15. Perform real on-chain upgrades in `execute_upgrade`

**Module:** `versioning` · **Labels:** `module:versioning`, `type:feature`, `priority:P1`

### Background

The versioning contract tracks versions, multi-sig approvals, migrations, feature flags,
and rollback — but `execute_upgrade` (`contracts/versioning/src/lib.rs`, line 467)
explicitly notes "The actual WASM upgrade is handled externally; this entrypoint performs
the version state transition." The contract's core promise (safe upgrades) therefore ends
at bookkeeping: after N-of-M approvals nothing ensures the on-chain code actually
changes, and nothing prevents the recorded "current version" from drifting from the
deployed bytecode.

### Direction

- Record the target contract id in `UpgradeProposal` (the deployable whose WASM will be
  replaced) and the new WASM hash (`BytesN<32>`) in `VersionMetadata`.
- In `execute_upgrade`, after the approval-threshold checks, perform the upgrade on-chain:
  for self-upgrade use `env.deployer().update_current_contract_wasm(&wasm_hash)`, or for
  remote targets call the target's own upgrade entry point via a client. The WASM blob
  must already be pinned on-chain (deploy a contract from the wasm via
  `env.deployer().upload_wasm` in a prior step, or accept the hash and require it exists).
- Gate on compatibility: run `migration::check_compatibility` before updating and
  `execute_migration` after, storing the `MigrationRecord` (both already exist).
- Keep `rollback` symmetric: it should re-run the previous version's pinned hash through
  the same update path (with its own multi-sig or admin gate — pick one and document).
- Emit `UPGRADE_EXECUTED { contract, old_hash, new_hash }` and record an audit entry;
  verify post-conditions (`env.deployer().current_wasm_hash` matches) before marking
  `Executed`.

### Acceptance criteria

- [ ] A test uploads two WASM variants, runs propose → approve ×threshold → execute, and
      asserts the live contract's wasm hash changed to the new one.
- [ ] `rollback` restores the previous hash and is blocked without its required
      authorization.
- [ ] Compatibility failure aborts before any code change; the proposal stays in a
      retryable state.
- [ ] Post-condition check: execute fails (typed `UpgradeVerificationFailed`) if the
      on-chain hash does not match after update.
- [ ] Governance tie-in documented: how issue #13's `ProtocolUpgrade` action reaches this
      entry point.

---

## 16. Cross-contract subscriber dispatch for the events contract

**Module:** `events` · **Labels:** `module:events`, `type:feature`, `priority:P1`

### Background

The subscription system (`contracts/events/src/subscriptions.rs`) already supports
filters, immediate vs batch delivery, exponential backoff (1 s → 1 h cap), delivery
records, acks, and metrics — but delivery is pure bookkeeping: `process_event` marks a
subscription "delivered" without ever calling the subscriber. The contract advertises
event-driven automation ("AI analysis triggers", `TRIG_ADD`) while nothing is actually
notified cross-contract. The existing Soroban `publish` events reach off-chain indexers
only.

### Direction

- Extend `ManagedSubscription` with an optional `subscriber_contract: Option<Address>`
  and a documented callback interface (a single `on_event(event_id, event_type, payload)`
  signature; publish the expected type in the crate docs so integrators implement it).
- In `process_event`, for subscriptions with a contract: invoke the callback via
  `env.invoke_contract`, wrapped so that:
  - delivery succeeds → `DeliveryRecord { status: Delivered, attempts, delivered_at }`
    as today, plus the event marked `Notified` for the AI-trigger path;
  - the call fails or the callback panics → record `Failed` and schedule the retry with
    the existing `backoff_delay` (a subsequent `process_pending_deliveries(keeper, max)`
    entry point replays due retries).
- Bounded work per invocation: cap callbacks per `process_event` call (e.g. 10) and per
  retry sweep so a flood of subscribers cannot blow the instruction budget.
- `unsubscribe` must stop dispatch immediately; paused subscriptions skip callbacks but
  keep buffering per their batch window.
- Keep current no-callback behavior exactly as-is for plain subscribers.

### Acceptance criteria

- [ ] A test deploys a mock subscriber contract implementing the callback; events reach
      it with correct payload and the delivery record shows `Delivered`.
- [ ] A subscriber that returns an error or panics is recorded `Failed` and is retried at
      the computed backoff times; retries stop at the documented max attempts.
- [ ] Batching subscriptions receive one callback per flush with the batch payload
      (or per-event callbacks at flush time — pick and document).
- [ ] A paused/cancelled subscription receives no callbacks from the moment of the state
      change.
- [ ] Retry sweep `process_pending_deliveries` processes at most the cap per call and is
      callable by anyone (keeper model, like issue #3).
- [ ] Existing subscription tests pass unchanged.

---

## 17. Automatic circuit-breaker tripping from the pricefeed

**Module:** `emergency` · **Labels:** `module:emergency`, `type:integration`, `priority:P1`

### Background

The emergency contract's circuit breaker trips only when someone calls
`report_price_change` manually, and `notify` merely records notifications against
registered notifiers — there is no automatic feed ingestion and no outbound call to any
monitoring system. During a real price crash the breaker depends on a human remembering
to file the report, which defeats the purpose.

### Direction

- Add `set_pricefeed(admin, pricefeed_contract: Address)`; then
  `check_circuit_from_feed(asset)` (anyone callable, keeper-friendly) reads the current
  aggregated price and compares it against a stored reference price per asset.
- Track a rolling reference: on each healthy check, decay/refresh the reference price
  (e.g. exponential moving average with a configurable half-life, or a high-water mark
  over a configurable window — implement the high-water mark first; it is simpler and
  conservative for crash detection).
- Trip automatically when the drop exceeds `circuit_threshold_bps` (default 20%, already
  defined), reusing the existing trip/reset/lock-period machinery, incident logging, and
  severity levels — do not duplicate state.
- Optionally add a "price-watch" registration list so one keeper call sweeps all
  registered assets (`sweep_price_watches(max_checks)` with a bounded loop).
- Cross-contract notifiers: when a notifier is a contract address, invoke its
  `on_incident(severity, reason)` callback (mirror issue #16's bounded-callback rules);
  account notifiers keep today's record-only behavior.
- Coordinate pausing: expose `is_globally_paused()` so staking/trade can consult it
  (issue #4's pause hook).

### Acceptance criteria

- [ ] A scripted price drop of > threshold from the pricefeed client causes
      `circuit_breaker_tripped = true` without any manual `report_price_change` call.
- [ ] Drops below threshold refresh the reference and do not trip; repeated small drops
      that accumulate past the threshold over multiple checks trip correctly.
- [ ] Reset still requires the existing guardian/admin authorization and lock period.
- [ ] `sweep_price_watches` bounds work (max_checks respected) and is callable by anyone.
- [ ] A contract notifier receives the incident callback; a failed callback does not
      roll back the trip.
- [ ] Existing emergency tests pass unchanged (no pricefeed configured ⇒ manual path
      only).

---

## 18. On-chain ed25519 verification entrypoint for signed audit exports

**Module:** `audit` · **Labels:** `module:audit`, `type:feature`, `priority:P2`

### Background

`contracts/audit/src/signing.rs` builds `SignedExport` (entries + SHA-256 digest +
ed25519 signature) but documents that verification is off-chain only, because Soroban's
`env.crypto().ed25519_verify` *panics* on invalid signatures, aborting any transaction
that tries to verify untrusted data. The result: an auditor cannot prove, in a single
on-chain transaction, that an export matches the chain's hash chain without risking a
revert on bad input.

### Direction

- Add `try_verify_export(export: SignedExport) -> Symbol` that:
  1. Recomputes the digest via the existing `serialize_entries` / `compute_digest`,
  2. compares it to `export.digest` (mismatch → typed `DigestMismatch` error *before*
     any signature work),
  3. only then calls `ed25519_verify`, letting the panic path serve as the
     "invalid signature" outcome for verifiers who want fail-fast semantics.
- Add `verify_export_against_chain(export)` for the stronger property: recompute the
  chain hash for `export.entries` via `checksum::chain_hash` and require it to link onto
  the contract's current `integrity_head` (or a stored historical head), proving the
  entries are genuine ledger entries — not just internally consistent.
- Add `is_signature_valid(export) -> bool` off-switch documentation: if a boolean
  API is truly needed, first compare digests on-chain (no crypto) and defer signature
  verification to a host-side check; document why.
- Register a trusted signer set (`add_trusted_signer(admin, pubkey)`,
  `remove_trusted_signer`) and have the verify entry points optionally require the
  signer to be registered.

### Acceptance criteria

- [ ] A valid export verifies on-chain; the digest-mismatch case returns a typed error
      *without* reaching `ed25519_verify` (proven by test — no panic).
- [ ] `verify_export_against_chain` accepts a genuine export and rejects one with a
      single altered field.
- [ ] Signer-set enforcement is tested for both allowed and unknown signers.
- [ ] Existing `compute_export_digest` / `verify_export` off-chain helpers remain
      unchanged; all current tests pass.
- [ ] `docs/API_REFERENCE.md` documents the three entry points and the panic semantics.

---

## 19. Real reward payouts for gamification (token-backed reward pool)

**Module:** `gamification` · **Labels:** `module:gamification`, `type:feature`, `priority:P2`

### Background

The gamification contract mints tier rewards (`REWARD_BRONZE` … `REWARD_PLATINUM`) and
challenge prizes as internal "virtual tokens" against `RWD_POOL` / `RWD_DIST` storage.
`fund_reward_pool` accepts an internal amount, so distributed rewards are not backed by
any asset — fine for a points system, but the API (`fund_reward_pool`,
`distribute_tier_reward`, `get_reward_distributed`) reads like a treasury, and the
integration tests treat it as one.

### Direction

- Introduce `fund_reward_pool(admin, token: Address, amount)` that performs a real
  token transfer into the contract and records the pool per token; keep a
  `Points` pseudo-token for virtual scoring if desired, explicitly separated.
- `distribute_tier_reward` / challenge payouts pay from the real pool when one is
  funded for the user's reward token: transfer out on claim, with
  `claim_rewards(user, token)` doing the outbound transfer (check-effects-first).
- Graceful dual mode: if no token pool is funded, behave exactly as today (virtual);
  document both modes and add a `get_reward_pool(token)` reader alongside the existing
  virtual-pool getter.
- Admin controls: `set_reward_rates(admin, per_tier)` to replace the hard-coded
  constants, and a per-user/per-period payout cap to bound exposure.
- Emit `REWARD_FUNDED`, `REWARD_PAID` (with token + amount) for indexers.

### Acceptance criteria

- [ ] Funding with a real token increases the contract's token balance by exactly the
      funded amount (mocked token test).
- [ ] Tier and challenge payouts transfer real tokens when available; a claim when the
      real pool is short fails closed with `InsufficientRewardPool` and leaves state
      unchanged.
- [ ] Virtual mode behavior is unchanged (existing tests pass untouched).
- [ ] Payout caps are enforced and admin-adjustable within documented bounds.
- [ ] `get_reward_distributed` reports real-token payouts separately from virtual ones.

---

## 20. Replace placeholder integration tests and harden CI

**Module:** cross-cutting · **Labels:** `area:ci`, `type:hardening`, `priority:P0`

### Background

Two systemic gaps undermine every other issue in this list:

1. **CI is effectively format-only.** `.github/workflows/rust-checks.yml` runs
   `cargo fmt --all -- --check`; the build, clippy, and test steps are commented out
   (because the empty `payments` crate — issue #1 — breaks workspace-wide builds).
   Nothing prevents a broken commit from landing.
2. **Integration tests are stubs.** `tests/integration_tests.rs` opens with six tests
   that only `println!` ("Rebalancing workflow test", etc.) — they pass while testing
   nothing. Only the fee/gamification/staking sections below them contain real
   cross-contract coverage.

### Direction

- **CI:** split the workflow into per-crate jobs (a matrix over the workspace members)
  so one broken crate doesn't hide others: `cargo build -p <crate>`,
  `cargo clippy -p <crate> -- -D warnings`, `cargo test -p <crate>` for every member,
  plus `cargo build --workspace` once issue #1 lands. Add a
  `cargo fmt --all -- --check` (keep), a WASM release build per contract
  (`soroban contract build` or `cargo build --target wasm32-unknown-unknown --release`),
  and a wasm artifact-size budget job that fails on >25 KB per contract (tune the
  number; `opt-level = "z"` + LTO is already configured).
- **Integration tests:** replace the six `println!` stubs with real end-to-end
  scenarios using `Env::default()` + `mock_all_auths`:
  - `test_cross_contract_interaction` → audit-sink flow: rebalancing (or staking) emits
    an audit event and the audit contract's chain hash advances (`integrity_head`).
  - `test_rebalancing_workflow` → set allocation → drift → simulate → execute
    (unblocked by issue #2; until then assert the plan + simulation outputs).
  - `test_staking_and_alerts` → stake with a threshold config, breach it, assert the
    alert history and alert event.
  - `test_event_emission` → emit_event → subscription filter → delivery record
    (mirror the real coverage that already exists under `contracts/events/tests/`).
  - `test_error_handling` → assert typed error codes across modules for the canonical
    failure cases (unauthorized, not-initialized, invalid amount).
  - `test_access_control` → RBAC grant/expire/revoke against the rebalancing contract's
    permission checks.
- Add a workflow step that fails when any test file matches the "placeholder" pattern
  (`fn test_.*\(\) \{[\s\S]*println!\('` style guard) to prevent regression — or a
  simple grep-based check in CI.

### Acceptance criteria

- [ ] CI runs build + clippy (`-D warnings`) + tests for every workspace crate on every
      PR; the workflow file contains no commented-out check steps.
- [ ] A deliberately broken test or clippy lint in a PR demonstrably fails CI (verified
      once on a scratch branch).
- [ ] All six placeholder tests in `tests/integration_tests.rs` contain real assertions
      against real contract instances; the `println!`-only pattern is gone.
- [ ] At least one test exercises a genuine cross-contract call (audit sink) end to end.
- [ ] WASM build job produces artifacts for all contracts and enforces the size budget;
      budgets are documented in the workflow file.
- [ ] Total CI wall time stays under ~10 minutes (parallel matrix), noted in the PR.

---

### Suggested milestones

- **M1 (correctness):** #1, #6, #20 — get the build green and the test floor honest.
- **M2 (real money):** #4, #5, #2, #8 — stop mocking assets and execution.
- **M3 (autonomy):** #3, #12, #16, #17 — keeper paths so the protocol runs itself.
- **M4 (maturity):** #7, #9, #10, #11, #13, #15 — scalability and full lifecycle closure.
- **M5 (polish):** #14, #18, #19 — power-user and trust features.
