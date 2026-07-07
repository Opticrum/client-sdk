//! WASM bindings for browser usage.
//!
//! Exposes a `WasmSdk` class via `wasm-bindgen` that wraps the core
//! `OpticrumSdk<RpcClient>`. All complex types are serialized to/from
//! JSON via `serde-wasm-bindgen`.

use wasm_bindgen::prelude::*;

use ckb_cinnabar_calculator::rpc::RpcClient;
use opticrum_calculator::types::CompressedPubkey;

use crate::{
    dashboard::compute_dashboard, deadline::find_matches_near_exhaustion, sdk::OpticrumSdk,
};

/// WASM-friendly SDK for the Opticrum protocol.
///
/// Wraps the core SDK with JSON serialization for JavaScript interop.
/// All methods return `JsValue` (JSON) that can be parsed on the JS side.
#[wasm_bindgen]
pub struct WasmSdk {
    inner: OpticrumSdk<RpcClient>,
}

#[wasm_bindgen]
impl WasmSdk {
    /// Create a new SDK connected to a custom CKB node.
    ///
    /// `ckb_rpc_url` — HTTP URL of the CKB RPC endpoint.
    /// `indexer_url` — HTTP URL of the CKB Indexer (defaults to RPC URL if empty).
    #[wasm_bindgen(constructor)]
    pub fn new(ckb_rpc_url: &str, indexer_url: Option<String>) -> WasmSdk {
        let indexer = indexer_url.as_deref().unwrap_or(ckb_rpc_url);
        let rpc = RpcClient::new(ckb_rpc_url, Some(indexer));
        WasmSdk {
            inner: OpticrumSdk::new(rpc),
        }
    }

    /// Create an SDK pre-configured for CKB testnet.
    #[wasm_bindgen]
    pub fn new_testnet() -> WasmSdk {
        WasmSdk {
            inner: OpticrumSdk::new(RpcClient::new_testnet()),
        }
    }

    /// Create an SDK pre-configured for CKB mainnet.
    #[wasm_bindgen]
    pub fn new_mainnet() -> WasmSdk {
        WasmSdk {
            inner: OpticrumSdk::new(RpcClient::new_mainnet()),
        }
    }

    // -----------------------------------------------------------------------
    // Read operations
    // -----------------------------------------------------------------------

    /// Get the current chain tip block number.
    #[wasm_bindgen]
    pub async fn get_tip_block(&self) -> Result<JsValue, JsValue> {
        let tip = self.inner.get_tip_block().await.map_err(to_js_err)?;
        Ok(JsValue::from_f64(tip as f64))
    }

    /// Scan live Order cells, optionally filtered by buyer Fiber pubkey.
    ///
    /// `fiber_pubkey_hex` — optional 66-char hex-encoded compressed pubkey.
    /// Returns JSON array of `OrderSummary` objects.
    #[wasm_bindgen]
    pub async fn scan_orders(&self, fiber_pubkey_hex: Option<String>) -> Result<JsValue, JsValue> {
        let pk = fiber_pubkey_hex.map(|h| parse_pubkey(&h)).transpose()?;
        let orders = self.inner.scan_orders(pk).await.map_err(to_js_err)?;
        let summaries: Vec<crate::types::OrderSummary> = orders
            .iter()
            .map(crate::dashboard::summarize_order)
            .collect();
        serde_wasm_bindgen::to_value(&summaries).map_err(to_js_err)
    }

    /// Scan live Match cells, optionally filtered by buyer Fiber pubkey.
    #[wasm_bindgen]
    pub async fn scan_matches(&self, fiber_pubkey_hex: Option<String>) -> Result<JsValue, JsValue> {
        let pk = fiber_pubkey_hex.map(|h| parse_pubkey(&h)).transpose()?;
        let tip = self.inner.get_tip_block().await.map_err(to_js_err)?;
        let matches = self.inner.scan_matches(pk).await.map_err(to_js_err)?;
        let summaries: Vec<crate::types::MatchSummary> = matches
            .iter()
            .map(|m| crate::dashboard::summarize_match(m, tip))
            .collect();
        serde_wasm_bindgen::to_value(&summaries).map_err(to_js_err)
    }

    // -----------------------------------------------------------------------
    // Dashboard & Monitoring
    // -----------------------------------------------------------------------

    /// Compute aggregated dashboard statistics from on-chain data.
    ///
    /// Returns JSON `DashboardData` object.
    #[wasm_bindgen]
    pub async fn dashboard(&self) -> Result<JsValue, JsValue> {
        let data = compute_dashboard(self.inner.rpc(), None)
            .await
            .map_err(to_js_err)?;
        serde_wasm_bindgen::to_value(&data).map_err(to_js_err)
    }

    /// Find matches near exhaustion.
    ///
    /// `blocks_threshold` — matches exhausting within this many blocks
    /// are included (e.g., 50400 = ~7 days).
    /// Returns JSON array of `MatchDeadline` objects.
    #[wasm_bindgen]
    pub async fn find_expiring_matches(&self, blocks_threshold: u32) -> Result<JsValue, JsValue> {
        let tip = self.inner.get_tip_block().await.map_err(to_js_err)?;
        let deadlines =
            find_matches_near_exhaustion(self.inner.rpc(), tip, blocks_threshold as u64, None)
                .await
                .map_err(to_js_err)?;
        serde_wasm_bindgen::to_value(&deadlines).map_err(to_js_err)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_pubkey(hex: &str) -> Result<CompressedPubkey, JsValue> {
    let bytes = hex::decode(hex).map_err(|e| JsValue::from_str(&format!("invalid hex: {e}")))?;
    CompressedPubkey::from_slice(&bytes)
        .map_err(|e| JsValue::from_str(&format!("invalid pubkey: {e}")))
}

fn to_js_err(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}
