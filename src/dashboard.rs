//! Dashboard data aggregation from on-chain sources.
//!
//! All data is computed by scanning live Order and Match cells on-chain.
//! No server-side database or caching — pure on-chain aggregation.

use ckb_cinnabar_calculator::rpc::RPC;
use opticrum_calculator::{
    calculator::rent_per_block_to_annual_yield,
    config::CKB_DECIMAL,
    types::{CompressedPubkey, MatchInfo, OrderInfo},
};

use crate::{
    deadline::compute_match_deadline,
    error::SdkError,
    sdk::OpticrumSdk,
    types::{
        DashboardData, MatchDetail, MatchSummary, OrderDetail, OrderSummary, YieldDistribution,
    },
};

/// Compute full dashboard statistics from on-chain data.
///
/// Scans all live orders and matches, then computes aggregate metrics
/// including yield distributions, exhaustion projections, and recent
/// activity summaries.
pub async fn compute_dashboard<T: RPC>(
    rpc: &T,
    fiber_pubkey: Option<CompressedPubkey>,
) -> Result<DashboardData, SdkError> {
    let sdk = OpticrumSdk::new(rpc.clone());
    let tip_block = sdk.get_tip_block().await?;
    let orders = sdk.scan_orders(fiber_pubkey.clone()).await?;
    let matches = sdk.scan_matches(fiber_pubkey).await?;

    let total_orders = orders.len();
    let total_matches = matches.len();

    let (active, exhausted): (Vec<&MatchInfo>, Vec<&MatchInfo>) =
        matches.iter().partition(|m| !m.is_exhausted(tip_block));

    let total_capacity_locked: u64 = matches.iter().map(|m| m.ckb_capacity).sum();
    let total_orders_capacity: u64 = orders.iter().map(|o| o.ckb_capacity).sum();

    // Average rent-per-block across active matches
    let avg_shannons: f64 = if active.is_empty() {
        0.0
    } else {
        active
            .iter()
            .map(|m| m.match_data.shannons_per_block as f64)
            .sum::<f64>()
            / active.len() as f64
    };

    // Yield distribution + average yield
    let mut yield_dist = YieldDistribution::standard();
    let mut total_yield_bps: f64 = 0.0;
    let mut yield_count: usize = 0;

    for m in &active {
        // Use match capacity as proxy for channel capacity (see Design Notes)
        let capacity = std::cmp::max(m.ckb_capacity, 1);
        let annual_yield =
            rent_per_block_to_annual_yield(m.match_data.shannons_per_block, capacity);
        let bps = (annual_yield * 10_000.0) as u64;
        yield_dist.add(bps, m.ckb_capacity);
        total_yield_bps += annual_yield * 10_000.0;
        yield_count += 1;
    }

    let avg_annual_yield_bps = if yield_count > 0 {
        total_yield_bps / yield_count as f64
    } else {
        0.0
    };

    // Matches near exhaustion (within 7 days)
    let matches_near_exhaustion = active
        .iter()
        .filter(|m| {
            let blocks_remaining =
                super::deadline::projected_exhaustion_block(m).saturating_sub(tip_block);
            blocks_remaining > 0 && blocks_remaining <= 50_400 // 7 days
        })
        .count();

    // Build summaries
    let recent_orders: Vec<OrderSummary> =
        orders.iter().rev().take(10).map(summarize_order).collect();

    let recent_matches: Vec<MatchSummary> = matches
        .iter()
        .rev()
        .take(10)
        .map(|m| summarize_match(m, tip_block))
        .collect();

    Ok(DashboardData {
        tip_block,
        total_orders,
        total_matches,
        active_matches: active.len(),
        exhausted_matches: exhausted.len(),
        total_capacity_locked_shannons: total_capacity_locked,
        total_orders_capacity_shannons: total_orders_capacity,
        avg_shannons_per_block: avg_shannons,
        avg_annual_yield_bps,
        matches_near_exhaustion,
        recent_orders,
        recent_matches,
        yield_distribution: yield_dist,
    })
}

// ---------------------------------------------------------------------------
// Summary builders
// ---------------------------------------------------------------------------

/// Build an [`OrderSummary`] from an [`OrderInfo`].
pub fn summarize_order(order: &OrderInfo) -> OrderSummary {
    let annual_yield = rent_per_block_to_annual_yield(
        order.order_data.shannons_per_block,
        order.order_data.channel_capacity,
    );

    OrderSummary {
        outpoint: format!(
            "{}:{}",
            hex::encode(order.order_outpoint.tx_hash),
            order.order_outpoint.index
        ),
        channel_capacity_ckb: order.order_data.channel_capacity as f64 / CKB_DECIMAL as f64,
        shannons_per_block: order.order_data.shannons_per_block,
        annual_yield_bps: annual_yield * 10_000.0,
        has_fiber_address: order.fiber_address.is_some(),
        xudt_amount: order.xudt.as_ref().map(|x| x.amount).unwrap_or(0),
    }
}

/// Build a [`MatchSummary`] from a [`MatchInfo`].
pub fn summarize_match(match_info: &MatchInfo, tip_block: u64) -> MatchSummary {
    let capacity = std::cmp::max(match_info.ckb_capacity, 1);
    let annual_yield =
        rent_per_block_to_annual_yield(match_info.match_data.shannons_per_block, capacity);

    let baseline = if match_info.match_data.last_extraction_block == 0 {
        match_info.match_current_block
    } else {
        match_info.match_data.last_extraction_block
    };
    let blocks_since = tip_block.saturating_sub(baseline);
    let extractable = match_info.extraction_amount(tip_block);

    MatchSummary {
        outpoint: format!(
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
        annual_yield_bps: annual_yield * 10_000.0,
        remaining_capacity_ckb: match_info.ckb_capacity as f64 / CKB_DECIMAL as f64,
        last_extraction_block: match_info.match_data.last_extraction_block,
        blocks_since_extraction: blocks_since,
        extractable_now_ckb: extractable as f64 / CKB_DECIMAL as f64,
        is_exhausted: match_info.is_exhausted(tip_block),
        projected_exhaustion_block: super::deadline::projected_exhaustion_block(match_info),
    }
}

// ---------------------------------------------------------------------------
// Detail builders
// ---------------------------------------------------------------------------

/// Build a detailed view of an Order cell.
pub fn get_order_detail(order: &OrderInfo) -> OrderDetail {
    let annual_yield = rent_per_block_to_annual_yield(
        order.order_data.shannons_per_block,
        order.order_data.channel_capacity,
    );

    OrderDetail {
        outpoint: format!(
            "{}:{}",
            hex::encode(order.order_outpoint.tx_hash),
            order.order_outpoint.index
        ),
        fiber_pubkey: hex::encode(order.order_args.fiber_pubkey.as_bytes()),
        buyer_lock_hash: hex::encode(order.order_args.buyer_lock_hash),
        channel_capacity_ckb: order.order_data.channel_capacity as f64 / CKB_DECIMAL as f64,
        shannons_per_block: order.order_data.shannons_per_block,
        annual_yield_bps: annual_yield * 10_000.0,
        rent_capacity_ckb: order.ckb_capacity as f64 / CKB_DECIMAL as f64,
        fiber_address: order.fiber_address.clone(),
        xudt_amount: order.xudt.as_ref().map(|x| x.amount).unwrap_or(0),
        block_number: 0, // OrderInfo doesn't expose block_number directly
    }
}

/// Build a detailed view of a Match cell.
pub fn get_match_detail(match_info: &MatchInfo, tip_block: u64) -> MatchDetail {
    let capacity = std::cmp::max(match_info.ckb_capacity, 1);
    let annual_yield =
        rent_per_block_to_annual_yield(match_info.match_data.shannons_per_block, capacity);
    let deadline = compute_match_deadline(match_info, tip_block);

    let baseline = if match_info.match_data.last_extraction_block == 0 {
        match_info.match_current_block
    } else {
        match_info.match_data.last_extraction_block
    };
    let blocks_since = tip_block.saturating_sub(baseline);
    let extractable = match_info.extraction_amount(tip_block);

    MatchDetail {
        outpoint: format!(
            "{}:{}",
            hex::encode(match_info.match_outpoint.tx_hash),
            match_info.match_outpoint.index
        ),
        channel_outpoint: format!(
            "{}:{}",
            hex::encode(match_info.match_args.channel_outpoint.tx_hash),
            match_info.match_args.channel_outpoint.index
        ),
        seller_lock_hash: hex::encode(match_info.match_args.seller_lock_hash),
        buyer_lock_hash: hex::encode(match_info.match_args.order_args.buyer_lock_hash),
        shannons_per_block: match_info.match_data.shannons_per_block,
        annual_yield_bps: annual_yield * 10_000.0,
        remaining_capacity_ckb: match_info.ckb_capacity as f64 / CKB_DECIMAL as f64,
        last_extraction_block: match_info.match_data.last_extraction_block,
        match_creation_block: match_info.match_current_block,
        blocks_since_extraction: blocks_since,
        extractable_now_ckb: extractable as f64 / CKB_DECIMAL as f64,
        is_exhausted: match_info.is_exhausted(tip_block),
        projected_exhaustion_block: deadline.projected_exhaustion_block,
        health: deadline.health,
        xudt_amount: match_info.xudt.as_ref().map(|x| x.amount).unwrap_or(0),
    }
}
