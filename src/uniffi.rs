//! Uniffi FFI support for iOS (Swift) and Android (Kotlin).
//!
//! Provides flat FFI-safe record types and async functions that create
//! `RpcClient` internally. Designed to work with Uniffi's UDL-based
//! code generation — see `uniffi/opticrum_sdk.udl` for the interface
//! definition.
//!
//! ## Usage
//!
//! 1. Add `uniffi` to your consuming crate's dependencies
//! 2. Create a small wrapper that imports these functions
//! 3. Run `uniffi-bindgen` on the UDL file:
//!    ```sh
//!    uniffi-bindgen generate uniffi/opticrum_sdk.udl --language kotlin
//!    uniffi-bindgen generate uniffi/opticrum_sdk.udl --language swift
//!    ```

use ckb_cinnabar_calculator::rpc::RpcClient;
use opticrum_calculator::types::CompressedPubkey;

use crate::{
    dashboard::{compute_dashboard, summarize_match, summarize_order},
    deadline::find_matches_near_exhaustion,
    sdk::OpticrumSdk,
    types::MatchHealth,
};

// ---------------------------------------------------------------------------
// FFI-safe record types (flat structs, no generics, no lifetimes)
// ---------------------------------------------------------------------------

/// Outpoint: transaction hash (hex) + output index.
pub struct FfiOutPoint {
    pub tx_hash: String,
    pub index: u32,
}

/// Summary of a live Order cell.
pub struct FfiOrderSummary {
    pub outpoint: String,
    pub fiber_pubkey: String,
    pub buyer_lock_hash: String,
    pub channel_capacity_ckb: f64,
    pub shannons_per_block: u64,
    pub annual_yield_bps: f64,
    pub has_fiber_address: bool,
    pub fiber_address: Option<String>,
    pub xudt_amount: u128,
}

/// Summary of a live Match cell.
pub struct FfiMatchSummary {
    pub outpoint: String,
    pub channel_outpoint: String,
    pub shannons_per_block: u64,
    pub annual_yield_bps: f64,
    pub remaining_capacity_ckb: f64,
    pub last_extraction_block: u64,
    pub blocks_since_extraction: u64,
    pub extractable_now_ckb: f64,
    pub is_exhausted: bool,
    pub projected_exhaustion_block: u64,
}

/// Aggregated dashboard statistics.
pub struct FfiDashboardStats {
    pub tip_block: u64,
    pub total_orders: u32,
    pub total_matches: u32,
    pub active_matches: u32,
    pub exhausted_matches: u32,
    pub total_capacity_locked_ckb: f64,
    pub avg_shannons_per_block: f64,
    pub avg_annual_yield_bps: f64,
    pub matches_near_exhaustion: u32,
}

/// Match exhaustion deadline info.
pub struct FfiMatchDeadline {
    pub match_outpoint: String,
    pub channel_outpoint: String,
    pub shannons_per_block: u64,
    pub remaining_capacity_ckb: f64,
    pub last_extraction_block: u64,
    pub match_creation_block: u64,
    pub projected_exhaustion_block: u64,
    pub blocks_remaining: u64,
    pub estimated_hours_remaining: f64,
    /// "healthy", "warning", "critical", or "exhausted"
    pub health: String,
    pub extractable_now_ckb: f64,
}

// ---------------------------------------------------------------------------
// Async API functions (to be exposed via Uniffi UDL)
// ---------------------------------------------------------------------------

fn health_to_string(h: &MatchHealth) -> String {
    match h {
        MatchHealth::Healthy => "healthy".into(),
        MatchHealth::Warning => "warning".into(),
        MatchHealth::Critical => "critical".into(),
        MatchHealth::Exhausted => "exhausted".into(),
    }
}

fn parse_pubkey(hex: &str) -> Result<CompressedPubkey, String> {
    let bytes = hex::decode(hex).map_err(|e| format!("invalid hex: {e}"))?;
    CompressedPubkey::from_slice(&bytes).map_err(|e| e.to_string())
}

fn make_rpc(ckb_url: &str, indexer_url: &str) -> RpcClient {
    RpcClient::new(ckb_url, Some(indexer_url))
}

/// Scan live Order cells.
pub async fn ffi_scan_orders(
    ckb_rpc_url: String,
    indexer_url: String,
    fiber_pubkey_hex: Option<String>,
) -> Result<Vec<FfiOrderSummary>, String> {
    let rpc = make_rpc(&ckb_rpc_url, &indexer_url);
    let sdk = OpticrumSdk::new(rpc);

    let pk = fiber_pubkey_hex.as_deref().map(parse_pubkey).transpose()?;

    let orders = sdk.scan_orders(pk).await.map_err(|e| e.to_string())?;

    Ok(orders
        .iter()
        .map(|o| {
            let summary = summarize_order(o);
            FfiOrderSummary {
                outpoint: summary.outpoint,
                fiber_pubkey: hex::encode(o.order_args.fiber_pubkey.as_bytes()),
                buyer_lock_hash: hex::encode(o.order_args.buyer_lock_hash),
                channel_capacity_ckb: summary.channel_capacity_ckb,
                shannons_per_block: summary.shannons_per_block,
                annual_yield_bps: summary.annual_yield_bps,
                has_fiber_address: summary.has_fiber_address,
                fiber_address: o.fiber_address.clone(),
                xudt_amount: summary.xudt_amount,
            }
        })
        .collect())
}

/// Scan live Match cells.
pub async fn ffi_scan_matches(
    ckb_rpc_url: String,
    indexer_url: String,
    fiber_pubkey_hex: Option<String>,
) -> Result<Vec<FfiMatchSummary>, String> {
    let rpc = make_rpc(&ckb_rpc_url, &indexer_url);
    let sdk = OpticrumSdk::new(rpc);

    let pk = fiber_pubkey_hex.as_deref().map(parse_pubkey).transpose()?;

    let tip = sdk.get_tip_block().await.map_err(|e| e.to_string())?;
    let matches = sdk.scan_matches(pk).await.map_err(|e| e.to_string())?;

    Ok(matches
        .iter()
        .map(|m| {
            let summary = summarize_match(m, tip);
            FfiMatchSummary {
                outpoint: summary.outpoint,
                channel_outpoint: summary.channel_outpoint,
                shannons_per_block: summary.shannons_per_block,
                annual_yield_bps: summary.annual_yield_bps,
                remaining_capacity_ckb: summary.remaining_capacity_ckb,
                last_extraction_block: summary.last_extraction_block,
                blocks_since_extraction: summary.blocks_since_extraction,
                extractable_now_ckb: summary.extractable_now_ckb,
                is_exhausted: summary.is_exhausted,
                projected_exhaustion_block: summary.projected_exhaustion_block,
            }
        })
        .collect())
}

/// Compute aggregated dashboard statistics.
pub async fn ffi_dashboard(
    ckb_rpc_url: String,
    indexer_url: String,
) -> Result<FfiDashboardStats, String> {
    let rpc = make_rpc(&ckb_rpc_url, &indexer_url);
    let data = compute_dashboard(&rpc, None)
        .await
        .map_err(|e| e.to_string())?;

    Ok(FfiDashboardStats {
        tip_block: data.tip_block,
        total_orders: data.total_orders as u32,
        total_matches: data.total_matches as u32,
        active_matches: data.active_matches as u32,
        exhausted_matches: data.exhausted_matches as u32,
        total_capacity_locked_ckb: data.total_capacity_locked_ckb(),
        avg_shannons_per_block: data.avg_shannons_per_block,
        avg_annual_yield_bps: data.avg_annual_yield_bps,
        matches_near_exhaustion: data.matches_near_exhaustion as u32,
    })
}

/// Find matches near exhaustion.
pub async fn ffi_find_expiring_matches(
    ckb_rpc_url: String,
    indexer_url: String,
    blocks_threshold: u32,
) -> Result<Vec<FfiMatchDeadline>, String> {
    let rpc = make_rpc(&ckb_rpc_url, &indexer_url);
    let sdk = OpticrumSdk::new(rpc);
    let tip = sdk.get_tip_block().await.map_err(|e| e.to_string())?;

    let deadlines = find_matches_near_exhaustion(sdk.rpc(), tip, blocks_threshold as u64, None)
        .await
        .map_err(|e| e.to_string())?;

    Ok(deadlines
        .iter()
        .map(|d| FfiMatchDeadline {
            match_outpoint: d.match_outpoint.clone(),
            channel_outpoint: d.channel_outpoint.clone(),
            shannons_per_block: d.shannons_per_block,
            remaining_capacity_ckb: d.remaining_capacity_ckb,
            last_extraction_block: d.last_extraction_block,
            match_creation_block: d.match_creation_block,
            projected_exhaustion_block: d.projected_exhaustion_block,
            blocks_remaining: d.blocks_remaining,
            estimated_hours_remaining: d.estimated_hours_remaining,
            health: health_to_string(&d.health),
            extractable_now_ckb: d.extractable_now_ckb,
        })
        .collect())
}
