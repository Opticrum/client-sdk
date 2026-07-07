# Opticrum SDK

Client SDK for [Opticrum](https://github.com/ashuralyk/opticrum), a decentralized liquidity marketplace on the [Fiber Network](https://github.com/nervosnetwork/fiber) for [CKB](https://github.com/nervosnetwork/ckb).

## What It Is

Opticrum is a protocol that lets CKB holders earn yield by renting out channel capacity. Buyers post **Orders** — on-chain cells offering to pay rent for inbound liquidity. Sellers match those orders with pre-created Fiber channels, producing **Match** cells that accumulate linear rent over time. Sellers extract rent as it accrues; buyers can cancel unmatched orders or top up matched channels.

The SDK is a **pure-Rust client library** that wraps the core calculator into a simplified, unsigned-transaction API. It builds `TransactionSkeleton` structs — your application handles keys, signing, and broadcasting.

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
opticrum-sdk = { path = "path/to/opticrum-sdk" }
```

### Basic usage

```rust
use opticrum_sdk::sdk::OpticrumSdk;
use ckb_cinnabar_calculator::rpc::RpcClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect to a CKB node
    let rpc = RpcClient::new("https://testnet.ckbapp.dev", None);
    let sdk = OpticrumSdk::new(rpc);

    // Read chain state
    let tip = sdk.get_tip_block().await?;
    println!("Tip block: {tip}");

    let orders = sdk.scan_orders(None).await?;
    println!("Live orders: {}", orders.len());

    let matches = sdk.scan_matches(None).await?;
    println!("Live matches: {}", matches.len());

    Ok(())
}
```

## Build

### Library (all targets)

```bash
cargo build
```

### With CLI binary

```bash
cargo build --features cli
```

### WASM (browser)

Requires [wasm-pack](https://rustwasm.github.io/wasm-pack/) and the `wasm32-unknown-unknown` target:

```bash
rustup target add wasm32-unknown-unknown
cargo build --features wasm --target wasm32-unknown-unknown
```

For a publishable npm package:

```bash
wasm-pack build --features wasm --out-dir pkg
```

### Tests

Tests use `FakeRpcClient` — an in-memory chain backend. No CKB node or compiled contracts needed.

```bash
cargo test                              # all tests
cargo test -- --nocapture               # with debug output
cargo test deadline                     # specific test module
cargo test --test sdk_tests             # specific test file
```

The contract binary at `../opticrum/build/release/opticrum` is read once per test run to seed the fake chain.

### Lint

```bash
cargo clippy --all-targets --all-features
cargo fmt --check
```

## Architecture

```
opticrum-protocol/              byte-level types, no_std, molecule encoding
       ↓
opticrum/calculator/opticrum/   transaction assembly, cell scanning, yield math
       ↓
opticrum-sdk/  ← this crate     simplified API, dashboard aggregation, deadline monitoring
       ↓
WASM  /  Uniffi  /  CLI         platform bindings
```

The SDK is a **thin wrapper** over the calculator. It adds three things:

- **Aggregation** — scan the chain once and get `DashboardData` (totals, yield distribution, near-exhaustion counts)
- **Deadline awareness** — project when a match runs out of capacity, classify health (Healthy / Warning / Critical / Exhausted)
- **Platform reach** — the same operations are available on WASM (browser), Uniffi (iOS/Android), and a CLI binary

## API Overview

All operations are on `OpticrumSdk<T: RPC>`. Read operations work on all targets; write operations are gated off WASM since browsers can't sign secp256k1.

### Read operations (all targets)

| Method | Returns | Description |
|---|---|---|
| `get_tip_block()` | `u64` | Current chain tip block number |
| `scan_orders(pubkey?)` | `Vec<OrderInfo>` | All live Order cells, optionally filtered by buyer pubkey |
| `scan_matches(pubkey?)` | `Vec<MatchInfo>` | All live Match cells, optionally filtered by buyer pubkey |

### Write operations (native only, not WASM)

| Method | Returns | Description |
|---|---|---|
| `build_create_order(...)` | `TransactionSkeleton` | Post a new liquidity order |
| `build_cancel_order(...)` | `TransactionSkeleton` | Cancel an unmatched order (burn pattern) |
| `build_match_order(...)` | `TransactionSkeleton` | Match an order with a Fiber channel |
| `build_extract_rent(...)` | `TransactionSkeleton` | Extract accrued rent from a match |
| `build_update_match(...)` | `TransactionSkeleton` | Buyer injects or withdraws capacity |
| `build_destroy_match(...)` | `TransactionSkeleton` | Destroy an exhausted match (burn pattern) |

### Dashboard & monitoring

```rust
use opticrum_sdk::dashboard::compute_dashboard;
use opticrum_sdk::deadline::{find_matches_near_exhaustion, find_exhausted_matches};

// Aggregate on-chain statistics
let data = compute_dashboard(sdk.rpc(), None).await?;
println!("Total orders: {}", data.total_orders);
println!("Active matches: {}", data.active_matches);

// Find matches expiring within ~7 days (50400 blocks)
let urgent = find_matches_near_exhaustion(sdk.rpc(), tip, 50400, None).await?;
```

### Type summaries

The `dashboard` module provides presentation helpers:

```rust
use opticrum_sdk::dashboard::{summarize_order, summarize_match, get_order_detail, get_match_detail};

let summary = summarize_order(&order_info);
println!("Yield: {:.2}% APR", summary.annual_yield_bps / 100.0);
```

## Feature Flags

| Feature | Adds | Use case |
|---|---|---|
| `cli` | `clap`, `tokio`, CLI binary | Command-line chain inspection |
| `wasm` | `wasm-bindgen`, `serde-wasm-bindgen` | Browser / JavaScript |
| `uniffi` | FFI-safe record types + async functions | iOS (Swift) / Android (Kotlin) |

## Web Integration (WASM)

The `wasm` feature exposes a `WasmSdk` class via `wasm-bindgen`. All return values are JSON — parse them on the JS side.

### Build the WASM package

```bash
wasm-pack build --features wasm --out-dir pkg
```

### JavaScript / TypeScript usage

```js
import init, { WasmSdk } from './pkg/opticrum_sdk.js';

await init();

// Connect to testnet (or use new("url", indexer?) / new_mainnet())
const sdk = new WasmSdk.new_testnet();

// Read chain state
const tip = await sdk.get_tip_block();
console.log('Tip block:', tip);

// Scan orders (pass null for all, or a pubkey hex string to filter)
const orders = JSON.parse(await sdk.scan_orders(null));
console.log(`Found ${orders.length} orders`);

// Scan matches
const matches = JSON.parse(await sdk.scan_matches(null));
console.log(`Found ${matches.length} matches`);

// Dashboard
const dashboard = JSON.parse(await sdk.dashboard());
console.log('Total capacity locked:', dashboard.total_capacity_locked_ckb, 'CKB');

// Find matches expiring within ~7 days
const expiring = JSON.parse(await sdk.find_expiring_matches(50400));
for (const m of expiring) {
  console.log(`${m.match_outpoint}: ${m.health} (${m.blocks_remaining} blocks left)`);
}
```

### WASM limitations

- **Read-only.** `build_*` transaction builders are not available on WASM — browsers lack secp256k1 signing. Build transactions server-side or in a native app.
- **JSON serialization.** All complex types cross the WASM boundary as JSON strings via `serde-wasm-bindgen`. Parse with `JSON.parse()`.
- **No streaming.** Each method is an async request/response. For real-time updates, poll or use a server-side event source.

## Mobile Integration (Uniffi — iOS & Android)

The `uniffi` feature provides flat FFI-safe types and async functions for iOS (Swift) and Android (Kotlin) via [Uniffi](https://mozilla.github.io/uniffi-rs/) code generation.

### Project setup

Add `opticrum-sdk` to your consuming crate's `Cargo.toml` with the `uniffi` feature enabled. The UDL file is at `uniffi/opticrum_sdk.udl`.

### Generate bindings

```bash
# Kotlin (Android)
uniffi-bindgen generate uniffi/opticrum_sdk.udl --language kotlin --out-dir generated/kotlin

# Swift (iOS)
uniffi-bindgen generate uniffi/opticrum_sdk.udl --language swift --out-dir generated/swift
```

### FFI type reference

FFI records are flat structs with no generics or lifetimes — safe to pass across the FFI boundary.

| FFI Record | Fields |
|---|---|
| `FfiOrderSummary` | `outpoint`, `fiber_pubkey`, `buyer_lock_hash`, `channel_capacity_ckb`, `shannons_per_block`, `annual_yield_bps`, `has_fiber_address`, `fiber_address`, `xudt_amount` |
| `FfiMatchSummary` | `outpoint`, `channel_outpoint`, `shannons_per_block`, `annual_yield_bps`, `remaining_capacity_ckb`, `last_extraction_block`, `blocks_since_extraction`, `extractable_now_ckb`, `is_exhausted`, `projected_exhaustion_block` |
| `FfiDashboardStats` | `tip_block`, `total_orders`, `total_matches`, `active_matches`, `exhausted_matches`, `total_capacity_locked_ckb`, `avg_shannons_per_block`, `avg_annual_yield_bps`, `matches_near_exhaustion` |
| `FfiMatchDeadline` | `match_outpoint`, `channel_outpoint`, `shannons_per_block`, `remaining_capacity_ckb`, `last_extraction_block`, `match_creation_block`, `projected_exhaustion_block`, `blocks_remaining`, `estimated_hours_remaining`, `health`, `extractable_now_ckb` |

### FFI functions

Each function creates its own `RpcClient` internally — no generics cross the FFI boundary.

| Function | Returns | Description |
|---|---|---|
| `ffi_scan_orders(rpc_url, indexer_url, pubkey?)` | `Vec<FfiOrderSummary>` | Scan live orders |
| `ffi_scan_matches(rpc_url, indexer_url, pubkey?)` | `Vec<FfiMatchSummary>` | Scan live matches |
| `ffi_dashboard(rpc_url, indexer_url)` | `FfiDashboardStats` | Aggregated statistics |
| `ffi_find_expiring_matches(rpc_url, indexer_url, threshold)` | `Vec<FfiMatchDeadline>` | Matches near exhaustion |

### Swift usage sketch

```swift
// After generating bindings with uniffi-bindgen
let stats = try await ffiDashboard(
    ckbRpcUrl: "https://testnet.ckbapp.dev",
    indexerUrl: "https://testnet.ckbapp.dev"
)
print("Active matches: \(stats.activeMatches)")
print("Total CKB locked: \(stats.totalCapacityLockedCkb)")

let expiring = try await ffiFindExpiringMatches(
    ckbRpcUrl: "https://testnet.ckbapp.dev",
    indexerUrl: "https://testnet.ckbapp.dev",
    blocksThreshold: 50400
)
for match in expiring {
    print("\(match.matchOutpoint): \(match.health)")
}
```

### Kotlin usage sketch

```kotlin
// After generating bindings with uniffi-bindgen
suspend fun loadDashboard() {
    val stats = ffiDashboard(
        "https://testnet.ckbapp.dev",
        "https://testnet.ckbapp.dev"
    )
    println("Active matches: ${stats.activeMatches}")
}

suspend fun checkExpiring() {
    val expiring = ffiFindExpiringMatches(
        "https://testnet.ckbapp.dev",
        "https://testnet.ckbapp.dev",
        50400u
    )
    expiring.forEach { deadline ->
        println("${deadline.matchOutpoint}: ${deadline.health}")
    }
}
```

## CLI

```bash
cargo run --features cli -- --rpc https://testnet.ckbapp.dev scan-orders
cargo run --features cli -- --rpc https://testnet.ckbapp.dev scan-matches
cargo run --features cli -- --rpc https://testnet.ckbapp.dev dashboard
cargo run --features cli -- --rpc https://testnet.ckbapp.dev monitor --blocks-threshold 50400
```

All CLI commands are read-only. The transaction builders are available in the library but not yet exposed as CLI subcommands.

## Design Decisions

**Generic over RPC.** `OpticrumSdk<T: RPC>` accepts `RpcClient` (production), `FakeRpcClient` (tests), or any custom implementation. The SDK has no hard network dependency.

**Unsigned by design.** Every `build_*` method returns a `TransactionSkeleton`. Inspect it, compose it, then sign and broadcast from your application.

**No database.** Dashboard data is computed on the fly by scanning live cells. No stored state, no cache invalidation, no migrations.

**Authorization lives on-chain.** The SDK has a client-side exhaustion guard (`SdkError::NotExhausted`), but access control is enforced by the contract verifier, not the SDK.

**Skip-chain invariant.** The SDK never defines its own copies of protocol types (`OrderInfo`, `MatchInfo`, `OrderArgs`, etc.). All types flow down from `opticrum-calculator` or `opticrum-protocol`.

## Related Projects

- [Opticrum Contracts & Calculator](https://github.com/nervosnetwork/fiber) — on-chain contracts + transaction assembly
- [Fiber Network](https://github.com/nervosnetwork/fiber) — CKB payment channel network
- [CKB](https://github.com/nervosnetwork/ckb) — Nervos Common Knowledge Base
