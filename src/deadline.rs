//! Match exhaustion deadline monitoring.
//!
//! Computes projected exhaustion blocks, remaining time, and health
//! classifications for Match cells. All data comes from on-chain cell
//! scanning — no server state required.

use ckb_cinnabar_calculator::rpc::RPC;
use opticrum_calculator::{
    config::CKB_DECIMAL,
    types::{CompressedPubkey, MatchInfo},
};

use crate::{
    chain::BLOCKS_PER_DAY,
    error::SdkError,
    types::{MatchDeadline, MatchHealth},
};

/// Blocks per week.
const BLOCKS_PER_WEEK: u64 = 7 * BLOCKS_PER_DAY; // 50_400

/// Compute the projected exhaustion block for a single Match cell.
///
/// The exhaustion block is the block where accumulated rent >= remaining
/// capacity. Formula:
///
/// ```text
/// baseline = max(last_extraction_block, match_creation_block)
/// remaining = ckb_capacity (or xudt_amount for xUDT matches)
/// blocks_to_exhaustion = remaining / shannons_per_block  (ceiling division)
/// exhaustion_block = baseline + blocks_to_exhaustion
/// ```
///
/// Returns `u64::MAX` if `shannons_per_block` is 0 (never exhausts).
pub fn projected_exhaustion_block(match_info: &MatchInfo) -> u64 {
    let rate = match_info.match_data.shannons_per_block;
    if rate == 0 {
        return u64::MAX;
    }

    let baseline = if match_info.match_data.last_extraction_block == 0 {
        match_info.match_current_block
    } else {
        match_info.match_data.last_extraction_block
    };

    let remaining = match match_info.xudt {
        Some(ref x) => x.amount as u64,
        None => match_info.ckb_capacity,
    };

    // Ceiling division: (remaining + rate - 1) / rate
    let blocks_to_exhaustion = remaining.saturating_add(rate - 1) / rate;
    baseline.saturating_add(blocks_to_exhaustion)
}

/// Classify match health based on remaining blocks.
///
/// Thresholds (assuming ~12s CKB block interval):
/// - 0 blocks remaining → Exhausted
/// - 1..=7,200 (1 day) → Critical
/// - 7,201..=50,400 (7 days) → Warning
/// - > 50,400 → Healthy
pub fn match_health(blocks_remaining: u64) -> MatchHealth {
    if blocks_remaining == 0 {
        MatchHealth::Exhausted
    } else if blocks_remaining <= BLOCKS_PER_DAY {
        MatchHealth::Critical
    } else if blocks_remaining <= BLOCKS_PER_WEEK {
        MatchHealth::Warning
    } else {
        MatchHealth::Healthy
    }
}

/// Build a [`MatchDeadline`] from a [`MatchInfo`] and the current tip block.
pub fn compute_match_deadline(match_info: &MatchInfo, tip_block: u64) -> MatchDeadline {
    let remaining = match match_info.xudt {
        Some(ref x) => x.amount as u64,
        None => match_info.ckb_capacity,
    };

    let is_exhausted = match_info.is_exhausted(tip_block);
    let extractable = match_info.extraction_amount(tip_block);
    let proj_block = projected_exhaustion_block(match_info);

    let blocks_remaining = if is_exhausted {
        0
    } else {
        proj_block.saturating_sub(tip_block)
    };

    // 12s per block → hours = blocks * 12 / 3600 = blocks / 300
    let estimated_hours = blocks_remaining as f64 / 300.0;

    let health = if is_exhausted {
        MatchHealth::Exhausted
    } else {
        match_health(blocks_remaining)
    };

    MatchDeadline {
        match_outpoint: format!(
            "{}:{}",
            hex::encode(match_info.match_outpoint.tx_hash),
            match_info.match_outpoint.index
        ),
        channel_outpoint: format!(
            "{}:{}",
            hex::encode(match_info.match_args.channel_outpoint.tx_hash),
            match_info.match_args.channel_outpoint.index
        ),
        shannons_per_block: match_info.match_data.shannons_per_block,
        remaining_capacity_ckb: remaining as f64 / CKB_DECIMAL as f64,
        last_extraction_block: match_info.match_data.last_extraction_block,
        match_creation_block: match_info.match_current_block,
        projected_exhaustion_block: proj_block,
        blocks_remaining,
        estimated_hours_remaining: estimated_hours,
        health,
        extractable_now_ckb: extractable as f64 / CKB_DECIMAL as f64,
    }
}

/// Find matches that are already exhausted.
pub async fn find_exhausted_matches<T: RPC>(
    rpc: &T,
    tip_block: u64,
    fiber_pubkey: Option<CompressedPubkey>,
) -> Result<Vec<MatchDeadline>, SdkError> {
    let sdk = crate::sdk::OpticrumSdk::new(rpc.clone());
    let matches = sdk.scan_matches(fiber_pubkey).await?;
    Ok(matches
        .iter()
        .filter(|m| m.is_exhausted(tip_block))
        .map(|m| compute_match_deadline(m, tip_block))
        .collect())
}

/// Find matches that will exhaust within the given block threshold.
///
/// Also includes matches that are already exhausted.
pub async fn find_matches_near_exhaustion<T: RPC>(
    rpc: &T,
    tip_block: u64,
    blocks_threshold: u64,
    fiber_pubkey: Option<CompressedPubkey>,
) -> Result<Vec<MatchDeadline>, SdkError> {
    let sdk = crate::sdk::OpticrumSdk::new(rpc.clone());
    let matches = sdk.scan_matches(fiber_pubkey).await?;
    Ok(matches
        .iter()
        .map(|m| compute_match_deadline(m, tip_block))
        .filter(|d| d.blocks_remaining <= blocks_threshold)
        .collect())
}

/// Sort deadlines by urgency (fewest blocks remaining first).
pub fn sort_by_urgency(deadlines: &mut [MatchDeadline]) {
    deadlines.sort_by_key(|d| d.blocks_remaining);
}
