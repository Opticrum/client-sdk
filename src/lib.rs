//! Opticrum SDK — client library for the Opticrum decentralized liquidity marketplace.
//!
//! Provides a simplified API for building unsigned transactions, scanning
//! on-chain orders and matches, monitoring match exhaustion deadlines, and
//! computing dashboard-level aggregated data.
//!
//! # Feature flags
//!
//! - `cli`: Enables the `opticrum-cli` binary (requires tokio + clap)
//! - `wasm`: Enables wasm-bindgen bindings for browser usage (requires wasm-pack)
//! - `uniffi`: Enables Uniffi FFI bindings for iOS/Android

pub mod chain;
pub mod dashboard;
pub mod deadline;
pub mod error;
pub mod sdk;
pub mod types;

#[cfg(feature = "wasm")]
pub mod wasm;

#[cfg(feature = "uniffi")]
pub mod uniffi;
