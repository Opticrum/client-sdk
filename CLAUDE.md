# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

This is the **Opticrum SDK** — a pure-Rust client library that wraps the `opticrum-calculator` crate into a simplified, unsigned-transaction API. It lives inside the Fiber monorepo at `fiber/opticrum-sdk/`.

The SDK builds unsigned `TransactionSkeleton` structs. It does **not** manage wallets, keys, or signing — the consumer balances, signs, and broadcasts independently.

## Build & Test Commands

```bash
# Build the library only (no optional features)
cargo build

# Build with the CLI binary (requires clap + tokio)
cargo build --features cli

# Build for WASM (requires wasm-pack and wasm32 target)
cargo build --features wasm --target wasm32-unknown-unknown

# Run all tests (uses FakeRpcClient — no CKB node needed)
cargo test

# Run a single test by name
cargo test test_get_tip_block

# Run all tests in a specific test file
cargo test --test sdk_tests

# Run tests matching a pattern
cargo test deadline

# Run with output shown (for println/debug)
cargo test -- --nocapture

# Lint (include tests — default only checks lib/bin)
cargo clippy --all-targets --all-features
cargo fmt --check

# Fix common clippy warnings automatically
cargo clippy --all-targets --all-features --fix
```

Tests use `FakeRpcClient` (from `ckb-cinnabar-calculator`), a deterministic in-memory chain backend — no CKB node or compiled contract binary is required. The contract binary at `../opticrum/build/release/opticrum` is read once per test run to seed the fake chain cells.

## Architecture

The SDK is a thin wrapper layer — it delegates all protocol logic to `opticrum-calculator` and all transaction assembly to `ckb-cinnabar-calculator`. It adds aggregation, summarization, deadline monitoring, and cross-platform bindings on top.

### Dependency chain

```
opticrum-protocol/               (byte-level types, no_std)
       ↓
opticrum/calculator/opticrum/      (transaction assembly, scanning, yield helpers)
ckb-cinnabar/calculate/            (TransactionCalculator, skeleton, RPC trait)
       ↓
opticrum-sdk/  ← this crate        (simplified API, dashboard, deadlines, WASM/FFI bindings)
```

### Module map

| Module | Purpose |
|---|---|
| `sdk` | `OpticrumSdk<T: RPC>` — the main entry point. Read ops (`get_tip_block`, `scan_orders`, `scan_matches`) are available on all targets. Write ops (`build_create_order`, `build_cancel_order`, `build_match_order`, `build_extract_rent`, `build_update_match`, `build_destroy_match`) are gated behind `#[cfg(not(target_arch = "wasm32"))]` — WASM only gets read operations since it lacks secp256k1 signing support. |
| `dashboard` | Pure on-chain aggregation — scans all live Order and Match cells and computes `DashboardData` (totals, yield distribution, near-exhaustion counts, recent summaries). Also provides `summarize_order`, `summarize_match`, `get_order_detail`, `get_match_detail` builders usable from WASM/Uniffi. No server state. |
| `deadline` | Match exhaustion projection. Computes `projected_exhaustion_block` via ceiling division of remaining capacity by rent rate, classifies `MatchHealth` into four tiers (Healthy/Warning/Critical/Exhausted), and provides bulk queries (`find_exhausted_matches`, `find_matches_near_exhaustion`, `sort_by_urgency`). |
| `types` | SDK-specific aggregation types (`DashboardData`, `OrderSummary`, `MatchSummary`, `MatchDeadline`, `OrderDetail`, `MatchDetail`, `YieldDistribution`, `MatchHealth`). All derive `Serialize`. |
| `error` | `SdkError` enum with variants: `Chain`, `Scan`, `Build`, `InvalidInput`, `AlreadyExhausted`, `NotExhausted`, `NotAuthorized`. Implements `std::error::Error` + `fmt::Display`. |
| `wasm` | (feature-gated) `WasmSdk` class via `wasm-bindgen` — wraps `OpticrumSdk<RpcClient>` with JSON serialization for JavaScript interop. Exposes constructors (`new`, `new_testnet`, `new_mainnet`) and methods (`get_tip_block`, `scan_orders`, `scan_matches`, `dashboard`, `find_expiring_matches`). All complex types are serialized via `serde-wasm-bindgen`. |
| `uniffi` | (feature-gated) FFI-safe flat record types (`FfiOrderSummary`, `FfiMatchSummary`, `FfiDashboardStats`, `FfiMatchDeadline`) and async API functions (`ffi_scan_orders`, `ffi_scan_matches`, `ffi_dashboard`, `ffi_find_expiring_matches`) for iOS (Swift) and Android (Kotlin) via Uniffi code generation. Each function creates its own `RpcClient` internally — no generics cross the FFI boundary. |

### Feature flags

| Feature | Dependencies | Effect |
|---|---|---|
| `default` | (none) | Library only, all targets |
| `cli` | `clap`, `tokio` | Enables the `opticrum-cli` binary |
| `wasm` | `wasm-bindgen`, `wasm-bindgen-futures`, `js-sys`, `serde-wasm-bindgen` | Enables `pub mod wasm` + WASM bindings |
| `uniffi` | (none — consumer adds `uniffi` crate) | Enables `pub mod uniffi` + FFI record types |

The crate type is `["lib", "cdylib"]` — `cdylib` is needed for `wasm-bindgen` (WASM) and for iOS/Android (Uniffi) to produce a shared library.

### WASM target limitations

- **No write operations.** The `build_*` methods live in a `#[cfg(not(target_arch = "wasm32"))]` impl block. On WASM, `OpticrumSdk` only exposes `get_tip_block`, `scan_orders`, and `scan_matches`.
- **No address/transaction types exposed.** WASM methods return JSON via `serde-wasm-bindgen`, using the SDK summary types (`OrderSummary`, `MatchSummary`, `DashboardData`, `MatchDeadline`) rather than raw protocol types.

### Uniffi pattern

The Uniffi module creates its own `RpcClient` internally (via URL strings) so the FFI boundary is flat — no generics, no lifetimes, no `RPC` trait. Each function:
1. Parses its string args, creates an `RpcClient`
2. Wraps it in `OpticrumSdk`, calls the core SDK
3. Maps results into `Ffi*` record types and returns `Result<T, String>`

**No long namespace chains in function bodies.** Never write `ckb_cinnabar_calculator::re_exports::ckb_jsonrpc_types::Uint32` — import the type at the top of the file and use only the short name (`Uint32`). Similarly prefer `H256` over `ckb_types::H256`, `PackedCellOutput` over `ckb_types::packed::CellOutput`, etc. Use type aliases at the import level to disambiguate when needed (e.g. `CellOutput as PackedCellOutput`).

**No `use` inside function bodies.** All `use` statements go at the top of the file.

### Key design decisions

1. **Generic over RPC.** `OpticrumSdk<T: RPC>` works with `RpcClient` (production HTTP), `FakeRpcClient` (testing), or custom implementations. Never hardcode the RPC backend.

2. **No wallet/key management.** The SDK only builds unsigned skeletons. Signing, fee calculation, and broadcasting belong to the consuming application or server.

3. **Authorization is on-chain.** `build_destroy_match` has a client-side exhaustion guard (`SdkError::NotExhausted`), but the SDK does not enforce seller-only authorization — that's checked by the contract verifier on-chain.

4. **All dashboard data comes from on-chain scanning.** There is no database or cache. `compute_dashboard` calls `scan_orders` + `scan_matches`, computes aggregates in memory, and returns.

5. **Rent properties flow down from the calculator.** `rent_per_block_to_annual_yield`, `CKB_DECIMAL`, `extraction_amount`, and `is_exhausted` are all imported from `opticrum_calculator`. The SDK only adds presentation and aggregation.

### The skip-chain invariant

When a type or constant exists in `opticrum-calculator` or `opticrum-protocol`, the SDK must **re-export or delegate** to it — never define its own copy. The types in `opticrum-sdk::types` are *aggregation and presentation* types only (summaries, dashboard data, yield buckets, FFI records). If it's a protocol type (`OrderInfo`, `MatchInfo`, `OrderArgs`, `MatchArgs`, `OrderData`, `MatchData`, `OutPoint`), it comes from upstream.

## Test Structure

```
tests/
├── common/mod.rs       # FakeRpcClient setup, test data builders, cell seeding helpers
├── sdk_tests.rs        # Core read/write operations
├── lifecycle_tests.rs  # Full state-machine lifecycle (create → cancel, create → match → extract)
├── dashboard_tests.rs  # Aggregation: summaries, detail views, yield distribution
├── deadline_tests.rs   # Exhaustion math: projections, health classification, edge cases
└── error_tests.rs      # Error paths: exhaustion guard, SDK error display
```

Shared helpers in `tests/common/mod.rs` provide:
- `test_rpc()` — pre-seeded `FakeRpcClient` with always-success + Opticrum contract celldeps
- `test_address()` / `seller_address()` — always-success lock addresses (blank args = buyer, `[0x01]` = seller)
- `test_order_args()`, `test_order_data()`, `test_match_args()`, `test_match_data()` — test data builders
- `seed_order()`, `seed_user_cell()`, `seed_seller_cell()`, `seed_channel_cell()`, `seed_header()` — in-memory cell/header seeding

Tests that require async use `#[tokio::test]`; pure-data tests use `#[test]`.

## CLI Usage

```bash
cargo run --features cli -- --rpc https://testnet.ckbapp.dev scan-orders
cargo run --features cli -- --rpc https://testnet.ckbapp.dev scan-matches
cargo run --features cli -- --rpc https://testnet.ckbapp.dev dashboard
cargo run --features cli -- --rpc https://testnet.ckbapp.dev monitor --blocks-threshold 50400
```

The CLI connects to a CKB node via `--rpc` and an optional `--indexer` (defaults to the RPC URL). All commands are read-only — the transaction builders are not yet exposed as CLI commands.
