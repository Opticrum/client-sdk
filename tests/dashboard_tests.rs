//! Tests for dashboard data aggregation from on-chain sources.

use opticrum_sdk::{
    dashboard::{
        compute_dashboard, get_match_detail, get_order_detail, summarize_match, summarize_order,
    },
    types::{DashboardData, MatchHealth, YieldBucket, YieldDistribution},
};

mod common;
use common::*;
use opticrum_calculator::types::{MatchArgs, MatchData, MatchInfo, OrderArgs};

// ---------------------------------------------------------------------------
// YieldDistribution
// ---------------------------------------------------------------------------

#[test]
fn test_yield_distribution_buckets() {
    let mut dist = YieldDistribution::standard();

    // 0.5% → 0-1% bucket
    dist.add(50, 100_000_000_000);
    // 2% → 1-3% bucket
    dist.add(200, 50_000_000_000);
    // 4% → 3-5% bucket
    dist.add(400, 25_000_000_000);
    // 7% → 5-10% bucket
    dist.add(700, 10_000_000_000);
    // 15% → 10%+ bucket
    dist.add(1500, 5_000_000_000);

    assert_eq!(dist.buckets[0].count, 1); // 0-1%
    assert_eq!(dist.buckets[1].count, 1); // 1-3%
    assert_eq!(dist.buckets[2].count, 1); // 3-5%
    assert_eq!(dist.buckets[3].count, 1); // 5-10%
    assert_eq!(dist.buckets[4].count, 1); // 10%+

    // Check capacities
    assert_eq!(dist.buckets[0].total_capacity_shannons, 100_000_000_000);
    assert_eq!(dist.buckets[1].total_capacity_shannons, 50_000_000_000);
    assert_eq!(dist.buckets[2].total_capacity_shannons, 25_000_000_000);
    assert_eq!(dist.buckets[3].total_capacity_shannons, 10_000_000_000);
    assert_eq!(dist.buckets[4].total_capacity_shannons, 5_000_000_000);
}

#[test]
fn test_yield_distribution_boundary() {
    let mut dist = YieldDistribution::standard();

    // Exactly 100 bps = 1% → should go to 1-3% bucket (min inclusive, max exclusive)
    dist.add(100, 1000);
    assert_eq!(dist.buckets[1].count, 1); // 1-3%
    assert_eq!(dist.buckets[0].count, 0); // NOT in 0-1%

    // Exactly 1000 bps = 10% → should go to 10%+ bucket
    dist.add(1000, 2000);
    assert_eq!(dist.buckets[4].count, 1); // 10%+
}

#[test]
fn test_yield_distribution_empty() {
    let dist = YieldDistribution::standard();
    for bucket in &dist.buckets {
        assert_eq!(bucket.count, 0);
        assert_eq!(bucket.total_capacity_shannons, 0);
    }
}

// ---------------------------------------------------------------------------
// summarize_order
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_summarize_order_fields() {
    let order_args = test_order_args();
    let order_data = test_order_data();
    let order_info = seed_order(
        &mut test_rpc().await,
        &order_args,
        &order_data,
        RENT_CAPACITY,
    )
    .await;

    let summary = summarize_order(&order_info);
    assert!(!summary.outpoint.is_empty());
    assert!(summary.outpoint.contains(':'));
    assert!(summary.channel_capacity_ckb > 0.0);
    assert_eq!(summary.shannons_per_block, SHANNONS_PER_BLOCK);
    assert!(summary.annual_yield_bps > 0.0);
    assert!(!summary.has_fiber_address); // no fiber addr seeded
    assert_eq!(summary.xudt_amount, 0);
}

// ---------------------------------------------------------------------------
// summarize_match
// ---------------------------------------------------------------------------

#[test]
fn test_summarize_match_fields() {
    let order_args = OrderArgs::new(fiber_pubkey(), user_lock_hash());
    let ch_op = channel_outpoint();
    let match_args = MatchArgs::new(order_args, ch_op, seller_lock_hash());
    let match_data = MatchData::new(0, SHANNONS_PER_BLOCK);

    let match_info = MatchInfo {
        match_args,
        match_data,
        xudt: None,
        ckb_capacity: RENT_CAPACITY,
        match_outpoint: channel_outpoint(),
        match_current_block: MATCH_CREATED_BLOCK,
    };

    let summary = summarize_match(&match_info, MATCH_CREATED_BLOCK);
    assert!(!summary.outpoint.is_empty());
    assert!(summary.outpoint.contains(':'));
    assert!(summary.remaining_capacity_ckb > 0.0);
    assert!(!summary.is_exhausted);
    assert!(summary.projected_exhaustion_block > MATCH_CREATED_BLOCK);
    assert_eq!(summary.shannons_per_block, SHANNONS_PER_BLOCK);
}

// ---------------------------------------------------------------------------
// get_order_detail / get_match_detail
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_order_detail() {
    let order_args = test_order_args();
    let order_data = test_order_data();
    let order_info = seed_order(
        &mut test_rpc().await,
        &order_args,
        &order_data,
        RENT_CAPACITY,
    )
    .await;

    let detail = get_order_detail(&order_info);
    assert!(detail.outpoint.contains(':'));
    assert!(!detail.fiber_pubkey.is_empty());
    assert!(!detail.buyer_lock_hash.is_empty());
    assert!(detail.channel_capacity_ckb > 0.0);
    assert!(detail.rent_capacity_ckb > 0.0);
}

#[test]
fn test_get_match_detail() {
    let order_args = OrderArgs::new(fiber_pubkey(), user_lock_hash());
    let ch_op = channel_outpoint();
    let match_args = MatchArgs::new(order_args, ch_op, seller_lock_hash());
    let match_data = MatchData::new(0, SHANNONS_PER_BLOCK);

    let match_info = MatchInfo {
        match_args,
        match_data,
        xudt: None,
        ckb_capacity: RENT_CAPACITY,
        match_outpoint: channel_outpoint(),
        match_current_block: MATCH_CREATED_BLOCK,
    };

    let detail = get_match_detail(&match_info, MATCH_CREATED_BLOCK + 5000);
    assert!(!detail.outpoint.is_empty());
    assert!(!detail.channel_outpoint.is_empty());
    assert!(!detail.seller_lock_hash.is_empty());
    assert!(!detail.buyer_lock_hash.is_empty());
    assert_eq!(detail.last_extraction_block, 0);
    assert_eq!(detail.match_creation_block, MATCH_CREATED_BLOCK);
    assert!(detail.blocks_since_extraction > 0);
    // Should be healthy with 30 CKB at 1000 shannons/block
    // 30 CKB = 3_000_000_000 shannons → ~3M blocks to exhaust
    assert_eq!(detail.health, MatchHealth::Healthy);
}

// ---------------------------------------------------------------------------
// YieldBucket::total_capacity_ckb
// ---------------------------------------------------------------------------

#[test]
fn test_yield_bucket_total_capacity_ckb() {
    let mut bucket = YieldBucket::new("test", 0, Some(100));
    bucket.total_capacity_shannons = 200_000_000_000; // 2000 CKB
    let ckb = bucket.total_capacity_ckb();
    assert!((ckb - 2000.0).abs() < 0.01, "expected ~2000 CKB, got {ckb}");

    let empty = YieldBucket::new("empty", 0, None);
    assert_eq!(empty.total_capacity_ckb(), 0.0);
}

// ---------------------------------------------------------------------------
// DashboardData helper methods
// ---------------------------------------------------------------------------

#[test]
fn test_dashboard_data_helpers() {
    let data = DashboardData {
        tip_block: 1000,
        total_orders: 5,
        total_matches: 3,
        active_matches: 2,
        exhausted_matches: 1,
        total_capacity_locked_shannons: 300_000_000_000, // 3000 CKB
        total_orders_capacity_shannons: 150_000_000_000, // 1500 CKB
        avg_shannons_per_block: 500.0,
        avg_annual_yield_bps: 250.0,
        matches_near_exhaustion: 1,
        recent_orders: vec![],
        recent_matches: vec![],
        yield_distribution: YieldDistribution::standard(),
    };

    assert!((data.total_capacity_locked_ckb() - 3000.0).abs() < 0.01);
    assert!((data.total_orders_capacity_ckb() - 1500.0).abs() < 0.01);
}

// ---------------------------------------------------------------------------
// YieldDistribution: fallback-to-last-bucket
// ---------------------------------------------------------------------------

#[test]
fn test_yield_distribution_fallback_to_last_bucket() {
    let mut dist = YieldDistribution::standard();
    // 2000 bps = 20% → should land in "10%+" (last bucket, max_bps=None)
    dist.add(2000, 42_000);
    assert_eq!(dist.buckets[4].count, 1);
    assert_eq!(dist.buckets[4].total_capacity_shannons, 42_000);
    // Other buckets should be empty
    for i in 0..4 {
        assert_eq!(dist.buckets[i].count, 0);
    }
}

#[test]
fn test_yield_distribution_multiple_matches_same_bucket() {
    let mut dist = YieldDistribution::standard();
    dist.add(50, 1000); // 0.5% → 0-1%
    dist.add(75, 2000); // 0.75% → 0-1%
    assert_eq!(dist.buckets[0].count, 2);
    assert_eq!(dist.buckets[0].total_capacity_shannons, 3000);
}

// ---------------------------------------------------------------------------
// compute_dashboard — integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_compute_dashboard_empty_chain() {
    let rpc = test_rpc().await;
    let data = compute_dashboard(&rpc, None).await.unwrap();

    assert_eq!(data.total_orders, 0);
    assert_eq!(data.total_matches, 0);
    assert_eq!(data.active_matches, 0);
    assert_eq!(data.exhausted_matches, 0);
    assert_eq!(data.total_capacity_locked_shannons, 0);
    assert_eq!(data.total_orders_capacity_shannons, 0);
    assert_eq!(data.avg_shannons_per_block, 0.0);
    assert_eq!(data.avg_annual_yield_bps, 0.0);
    assert_eq!(data.matches_near_exhaustion, 0);
    assert!(data.recent_orders.is_empty());
    assert!(data.recent_matches.is_empty());
}

#[tokio::test]
async fn test_compute_dashboard_with_orders() {
    let mut rpc = test_rpc().await;

    // Seed 2 orders
    let order_args = test_order_args();
    let order_data = test_order_data();
    seed_order(&mut rpc, &order_args, &order_data, RENT_CAPACITY).await;
    seed_order(&mut rpc, &order_args, &order_data, RENT_CAPACITY).await;

    let data = compute_dashboard(&rpc, None).await.unwrap();

    assert_eq!(data.total_orders, 2);
    assert!(data.total_orders_capacity_shannons > 0);
    assert!(!data.recent_orders.is_empty());
    assert_eq!(data.recent_orders.len(), 2); // 2 orders, both in "recent 10"
}

#[tokio::test]
async fn test_compute_dashboard_with_orders_and_matches() {
    let mut rpc = test_rpc().await;

    // Seed orders
    let order_args = test_order_args();
    let order_data = test_order_data();
    seed_order(&mut rpc, &order_args, &order_data, RENT_CAPACITY).await;

    // Seed a match
    let ch_op = channel_outpoint();
    let match_args = MatchArgs::new(order_args.clone(), ch_op, seller_lock_hash());
    let match_data = MatchData::new(0, SHANNONS_PER_BLOCK);
    seed_match(
        &mut rpc,
        &match_args,
        &match_data,
        RENT_CAPACITY,
        MATCH_CREATED_BLOCK,
    )
    .await;

    let data = compute_dashboard(&rpc, None).await.unwrap();

    assert!(data.total_orders >= 1, "should have at least 1 order");
    assert!(data.total_matches >= 1, "should have at least 1 match");
    assert!(data.total_capacity_locked_shannons > 0);

    // The match we seeded is far from exhausted (30 CKB, 1000 shannons/block)
    assert!(data.active_matches >= 1);
    assert_eq!(data.exhausted_matches, 0);
    assert!(data.avg_shannons_per_block > 0.0);
}
