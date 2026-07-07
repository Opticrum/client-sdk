//! SDK-specific aggregation types.
//!
//! These types are used for dashboard data, match deadline monitoring, and
//! summarized views of on-chain state. All data is derived purely from
//! on-chain cell scanning — no server or database required.

use opticrum_calculator::config::CKB_DECIMAL;
use serde::Serialize;

// ---------------------------------------------------------------------------
// Dashboard
// ---------------------------------------------------------------------------

/// Aggregated on-chain dashboard statistics.
///
/// Computed by scanning all live Order and Match cells and computing
/// aggregate metrics. No server-side data — everything comes from the chain.
#[derive(Clone, Debug, Serialize)]
pub struct DashboardData {
    /// Current chain tip block number.
    pub tip_block: u64,
    /// Total number of live Order cells.
    pub total_orders: usize,
    /// Total number of live Match cells.
    pub total_matches: usize,
    /// Number of matches that are not yet exhausted.
    pub active_matches: usize,
    /// Number of matches that are already exhausted.
    pub exhausted_matches: usize,
    /// Sum of CKB capacity locked in match cells (shannons).
    pub total_capacity_locked_shannons: u64,
    /// Sum of CKB capacity in order cells (shannons).
    pub total_orders_capacity_shannons: u64,
    /// Average rent-per-block across all active matches.
    pub avg_shannons_per_block: f64,
    /// Average annual yield in basis points (e.g., 500 = 5.00%).
    pub avg_annual_yield_bps: f64,
    /// Matches projected to exhaust within ~7 days of blocks.
    pub matches_near_exhaustion: usize,
    /// Most recent orders (up to 10).
    pub recent_orders: Vec<OrderSummary>,
    /// Most recent matches (up to 10).
    pub recent_matches: Vec<MatchSummary>,
    /// Distribution of yields across yield buckets.
    pub yield_distribution: YieldDistribution,
}

impl DashboardData {
    /// Total capacity locked in CKB (human-readable).
    pub fn total_capacity_locked_ckb(&self) -> f64 {
        self.total_capacity_locked_shannons as f64 / CKB_DECIMAL as f64
    }

    /// Total orders capacity in CKB (human-readable).
    pub fn total_orders_capacity_ckb(&self) -> f64 {
        self.total_orders_capacity_shannons as f64 / CKB_DECIMAL as f64
    }
}

// ---------------------------------------------------------------------------
// Summaries
// ---------------------------------------------------------------------------

/// Human-readable summary of a single Order cell.
#[derive(Clone, Debug, Serialize)]
pub struct OrderSummary {
    /// Transaction hash (hex) + output index.
    pub outpoint: String,
    /// Minimum channel capacity requested by the buyer (CKB).
    pub channel_capacity_ckb: f64,
    /// Per-block rent rate in shannons.
    pub shannons_per_block: u64,
    /// Equivalent annual yield in basis points (e.g., 500 = 5.00%).
    pub annual_yield_bps: f64,
    /// Whether the order includes a Fiber node address for peer discovery.
    pub has_fiber_address: bool,
    /// xUDT amount (0 for CKB-only orders).
    pub xudt_amount: u128,
}

/// Human-readable summary of a single Match cell.
#[derive(Clone, Debug, Serialize)]
pub struct MatchSummary {
    /// Transaction hash (hex) + output index.
    pub outpoint: String,
    /// Channel outpoint (hex).hash:index.
    pub channel_outpoint: String,
    /// Per-block rent rate in shannons.
    pub shannons_per_block: u64,
    /// Equivalent annual yield in basis points.
    pub annual_yield_bps: f64,
    /// Remaining capacity available for extraction (CKB).
    pub remaining_capacity_ckb: f64,
    /// Block number of the last extraction (0 = never extracted).
    pub last_extraction_block: u64,
    /// Blocks elapsed since the last extraction.
    pub blocks_since_extraction: u64,
    /// Currently extractable amount (CKB).
    pub extractable_now_ckb: f64,
    /// Whether the match is already exhausted.
    pub is_exhausted: bool,
    /// Projected block number when the match will be exhausted.
    pub projected_exhaustion_block: u64,
}

// ---------------------------------------------------------------------------
// Yield distribution
// ---------------------------------------------------------------------------

/// Distribution of annual yields across matches.
#[derive(Clone, Debug, Serialize)]
pub struct YieldDistribution {
    pub buckets: Vec<YieldBucket>,
}

impl YieldDistribution {
    /// Create yield distribution with standard buckets.
    pub fn standard() -> Self {
        Self {
            buckets: vec![
                YieldBucket::new("0–1%", 0, Some(100)),
                YieldBucket::new("1–3%", 100, Some(300)),
                YieldBucket::new("3–5%", 300, Some(500)),
                YieldBucket::new("5–10%", 500, Some(1000)),
                YieldBucket::new("10%+", 1000, None),
            ],
        }
    }

    /// Add a match to the appropriate bucket.
    pub fn add(&mut self, annual_yield_bps: u64, capacity: u64) {
        for bucket in &mut self.buckets {
            if bucket.contains(annual_yield_bps) {
                bucket.count += 1;
                bucket.total_capacity_shannons += capacity;
                return;
            }
        }
        // Fallback: add to last bucket
        if let Some(last) = self.buckets.last_mut() {
            last.count += 1;
            last.total_capacity_shannons += capacity;
        }
    }
}

/// A single yield distribution bucket.
#[derive(Clone, Debug, Serialize)]
pub struct YieldBucket {
    /// Human-readable label (e.g., "3–5%").
    pub label: String,
    /// Minimum basis points (inclusive).
    pub min_bps: u64,
    /// Maximum basis points (exclusive). `None` = unbounded.
    pub max_bps: Option<u64>,
    /// Number of matches in this bucket.
    pub count: usize,
    /// Total capacity in this bucket (shannons).
    pub total_capacity_shannons: u64,
}

impl YieldBucket {
    pub fn new(label: &str, min_bps: u64, max_bps: Option<u64>) -> Self {
        Self {
            label: label.to_string(),
            min_bps,
            max_bps,
            count: 0,
            total_capacity_shannons: 0,
        }
    }

    fn contains(&self, bps: u64) -> bool {
        bps >= self.min_bps && self.max_bps.is_none_or(|max| bps < max)
    }

    /// Total capacity in CKB.
    pub fn total_capacity_ckb(&self) -> f64 {
        self.total_capacity_shannons as f64 / CKB_DECIMAL as f64
    }
}

// ---------------------------------------------------------------------------
// Match deadline / health
// ---------------------------------------------------------------------------

/// Health classification for a Match cell based on time until exhaustion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchHealth {
    /// More than 7 days until exhaustion.
    Healthy,
    /// 1–7 days until exhaustion.
    Warning,
    /// Less than 1 day until exhaustion.
    Critical,
    /// Already exhausted — can be destroyed.
    Exhausted,
}

/// Exhaustion status and projection for a single Match cell.
#[derive(Clone, Debug, Serialize)]
pub struct MatchDeadline {
    /// Match outpoint (hex string).
    pub match_outpoint: String,
    /// Channel outpoint (hex string).
    pub channel_outpoint: String,
    /// Per-block rent rate in shannons.
    pub shannons_per_block: u64,
    /// Remaining capacity (CKB).
    pub remaining_capacity_ckb: f64,
    /// Block of last extraction (0 = never extracted).
    pub last_extraction_block: u64,
    /// Block where the match was created.
    pub match_creation_block: u64,
    /// Projected block where the match becomes exhausted.
    pub projected_exhaustion_block: u64,
    /// Blocks remaining until exhaustion.
    pub blocks_remaining: u64,
    /// Estimated hours until exhaustion (blocks × 12 / 3600).
    pub estimated_hours_remaining: f64,
    /// Health classification.
    pub health: MatchHealth,
    /// Currently extractable amount (CKB).
    pub extractable_now_ckb: f64,
}

// ---------------------------------------------------------------------------
// Detail views
// ---------------------------------------------------------------------------

/// Detailed view of an Order cell (enriched beyond raw OrderInfo).
#[derive(Clone, Debug, Serialize)]
pub struct OrderDetail {
    pub outpoint: String,
    pub fiber_pubkey: String,
    pub buyer_lock_hash: String,
    pub channel_capacity_ckb: f64,
    pub shannons_per_block: u64,
    pub annual_yield_bps: f64,
    pub rent_capacity_ckb: f64,
    pub fiber_address: Option<String>,
    pub xudt_amount: u128,
    pub block_number: u64,
}

/// Detailed view of a Match cell (enriched beyond raw MatchInfo).
#[derive(Clone, Debug, Serialize)]
pub struct MatchDetail {
    pub outpoint: String,
    pub channel_outpoint: String,
    pub seller_lock_hash: String,
    pub buyer_lock_hash: String,
    pub shannons_per_block: u64,
    pub annual_yield_bps: f64,
    pub remaining_capacity_ckb: f64,
    pub last_extraction_block: u64,
    pub match_creation_block: u64,
    pub blocks_since_extraction: u64,
    pub extractable_now_ckb: f64,
    pub is_exhausted: bool,
    pub projected_exhaustion_block: u64,
    pub health: MatchHealth,
    pub xudt_amount: u128,
}
