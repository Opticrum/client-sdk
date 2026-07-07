# Opticrum SDK

Client SDK for [Opticrum](https://github.com/ashuralyk/opticrum), a decentralized liquidity marketplace on the [Fiber Network](https://github.com/nervosnetwork/fiber) for [CKB](https://github.com/nervosnetwork/ckb).

## What It Is

Opticrum is a protocol that lets CKB holders earn yield by renting out channel capacity. Buyers post **Orders** — on-chain cells offering to pay rent for inbound liquidity. Sellers match those orders with pre-created Fiber channels, producing **Match** cells that accumulate linear rent over time. Sellers extract rent as it accrues; buyers can cancel unmatched orders or top up matched channels.

The SDK is the client side of this protocol. It wraps the core calculator into a simplified API that **builds unsigned transactions**. It never touches keys, wallets, or signatures — your application handles that.

## Why It Exists

There are two ways to interact with Opticrum:

1. **Assemble transactions by hand** — pick the right cells, set the right lock scripts, pack the right molecule-encoded args, resolve type IDs, balance inputs and outputs. Error-prone and tightly coupled to internal contract layout.

2. **Use the SDK** — call `build_create_order(...)` and get back a balanced, unsigned `TransactionSkeleton` ready to sign.

The SDK exists so you never have to know how Order args are packed, which cell deps the contract needs, or how `ScriptEx::Reference` resolves to a live code cell. It encodes those invariants once and exposes intent-level functions instead.

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

The SDK is a **thin wrapper** over the calculator. All protocol logic — how orders become matches, how rent is computed, how the contract verifier expects cells to be arranged — lives in the calculator. The SDK adds three things:

- **Aggregation** — scan the chain once and get `DashboardData` (totals, yield distribution, near-exhaustion counts). No database, no cache, just an in-memory fold over live cells.
- **Deadline awareness** — project when a match runs out of capacity, classify its health (Healthy / Warning / Critical / Exhausted), sort by urgency. The math is ceiling division — the same formula the contract verifier uses.
- **Platform reach** — the same read operations are available on WASM (browser), Uniffi (iOS/Android), and a CLI binary, all from one codebase.

## Core Abstractions

**`OpticrumSdk<T: RPC>`** — the main entry point, generic over the chain backend. Read operations (`scan_orders`, `scan_matches`, `get_tip_block`) are available everywhere. Write operations (`build_create_order`, `build_cancel_order`, etc.) are gated off WASM since it lacks secp256k1 signing support.

**The skip-chain invariant** — the SDK never defines its own copies of protocol types. `OrderInfo`, `MatchInfo`, `OrderArgs`, `OrderData`, `MatchArgs`, `MatchData`, `OutPoint`, `Xudt` all flow down from `opticrum-calculator` or `opticrum-protocol`. If a type needs to change, it starts upstream and the SDK inherits the update automatically.

**Dashboard** — `compute_dashboard` is a pure function of on-chain state. Given an RPC handle and an optional pubkey filter, it scans all live Order and Match cells, folds them into `DashboardData`, and returns. There is no incremental state, no caching layer, no index. Every call sees the chain as it is right now.

**Deadline** — `projected_exhaustion_block` divides remaining capacity by rent rate. No timestamp math, no approximations — it's the exact same block-level arithmetic the on-chain verifier uses. `MatchHealth` maps remaining blocks to four tiers based on configurable thresholds.

## Design Choices

**Generic over RPC, not coupled to HTTP.** `OpticrumSdk<T: RPC>` accepts `RpcClient` (production reqwest client), `FakeRpcClient` (deterministic in-memory chain for tests), or any custom implementation. The SDK itself has no network dependency — it only knows the `RPC` trait.

**Unsigned by design.** Every `build_*` method returns a `TransactionSkeleton`. No private keys, no signing, no broadcasting. The consumer balances, signs, and sends. You can inspect the skeleton before committing, or compose it into larger transactions.

**No database.** Dashboard data is computed on the fly by scanning live cells. There is no stored state to drift from the chain, no cache invalidation to manage, no migration to run. The trade-off is latency — a full scan reads every live Order and Match cell — but for a protocol where cells number in the hundreds, this is the right default.

**Authorization lives on-chain.** The SDK has a client-side exhaustion guard (`SdkError::NotExhausted` prevents building a destroy transaction for a match that still has capacity), but access control — who can cancel, who can destroy — is enforced by the contract verifier, not the SDK.

## Platform Bindings

- **WASM** (`wasm` feature) — `WasmSdk` wraps `OpticrumSdk<RpcClient>` with JSON serialization via `serde-wasm-bindgen`. Read-only: the `build_*` methods are excluded from the WASM target since browsers can't sign secp256k1.
- **Uniffi** (`uniffi` feature) — flat FFI records (`FfiOrderSummary`, `FfiMatchSummary`, etc.) and async functions that create their own `RpcClient` internally. No generics cross the FFI boundary. Codegen target for Kotlin (Android) and Swift (iOS).
- **CLI** (`cli` feature) — `opticrum-cli` for quick chain inspection. Scan orders, compute dashboards, monitor match exhaustion. Read-only; transaction builders are available but not yet exposed as CLI commands (except cancel/destroy).

## Testing

Tests use `FakeRpcClient` — an in-memory chain backend seeded with the Opticrum contract binary. No CKB node, no network, no compiled contracts needed at test time. Every test starts from a known state and runs in milliseconds.

The lifecycle tests exercise the full state machine end-to-end: create → cancel, create → match → extract, match → exhaust → destroy.
