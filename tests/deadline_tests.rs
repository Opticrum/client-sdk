//! Tests for match exhaustion deadline computation and health classification.

use opticrum_calculator::types::{
    CompressedPubkey, MatchArgs, MatchData, MatchInfo, OrderArgs, OutPoint,
};
use opticrum_sdk::{
    deadline::{
        compute_match_deadline, find_exhausted_matches, find_matches_near_exhaustion, match_health,
        projected_exhaustion_block,
    },
    types::MatchHealth,
};

mod common;
use common::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn test_outpoint() -> OutPoint {
    OutPoint::new([0u8; 32], 0)
}

fn test_order_args() -> OrderArgs {
    OrderArgs::new(CompressedPubkey::new([0x02; 33]), [0u8; 32])
}

fn make_match(
    shannons_per_block: u64,
    ckb_capacity: u64,
    last_extraction_block: u64,
    match_current_block: u64,
) -> MatchInfo {
    MatchInfo {
        match_args: MatchArgs::new(test_order_args(), test_outpoint(), [0u8; 32]),
        match_data: MatchData {
            xudt_amount: 0,
            shannons_per_block,
            last_extraction_block,
        },
        xudt: None,
        ckb_capacity,
        match_outpoint: test_outpoint(),
        match_current_block,
    }
}

// ---------------------------------------------------------------------------
// projected_exhaustion_block
// ---------------------------------------------------------------------------

#[test]
fn test_projected_exhaustion_fresh_match() {
    // Fresh match: created at block 1000, shannons_per_block = 1000,
    // capacity = 30 CKB = 3_000_000_000 shannons
    let match_info = make_match(1000, 3_000_000_000, 0, 1000);
    // Baseline = match_current_block = 1000
    // blocks_to_exhaustion = ceiling(3_000_000_000 / 1000) = 3_000_000
    // exhaustion_block = 1000 + 3_000_000 = 3_001_000
    let proj = projected_exhaustion_block(&match_info);
    assert_eq!(proj, 1_000 + 3_000_000);
}

#[test]
fn test_projected_exhaustion_post_extraction() {
    // After extraction at block 5000, last_extraction_block = 5000
    let match_info = make_match(1000, 1_000_000_000, 5000, 1000);
    // Baseline = last_extraction_block = 5000
    // blocks_to_exhaustion = ceiling(1_000_000_000 / 1000) = 1_000_000
    // exhaustion_block = 5000 + 1_000_000 = 1_005_000
    let proj = projected_exhaustion_block(&match_info);
    assert_eq!(proj, 5_000 + 1_000_000);
}

#[test]
fn test_projected_exhaustion_zero_rate() {
    // Zero rate means never exhausts
    let match_info = make_match(0, 1_000_000_000, 0, 1000);
    let proj = projected_exhaustion_block(&match_info);
    assert_eq!(proj, u64::MAX);
}

#[test]
fn test_projected_exhaustion_ceiling_division() {
    // remaining = 100, rate = 30. blocks = ceil(100/30) = 4
    let match_info = make_match(30, 100, 0, 1000);
    let proj = projected_exhaustion_block(&match_info);
    assert_eq!(proj, 1000 + 4); // ceiling(100/30) = 4
}

#[test]
fn test_projected_exhaustion_exact_division() {
    // remaining = 100, rate = 25. blocks = 100/25 = 4
    let match_info = make_match(25, 100, 0, 1000);
    let proj = projected_exhaustion_block(&match_info);
    assert_eq!(proj, 1000 + 4);
}

#[test]
fn test_projected_exhaustion_large_capacity() {
    // Very large capacity, small rate
    let match_info = make_match(1, u64::MAX, 0, 0);
    let proj = projected_exhaustion_block(&match_info);
    assert_eq!(proj, u64::MAX); // saturating addition
}

// ---------------------------------------------------------------------------
// match_health
// ---------------------------------------------------------------------------

#[test]
fn test_match_health_exhausted() {
    assert_eq!(match_health(0), MatchHealth::Exhausted);
}

#[test]
fn test_match_health_critical_boundaries() {
    assert_eq!(match_health(1), MatchHealth::Critical);
    assert_eq!(match_health(3600), MatchHealth::Critical); // ~12 hours
    assert_eq!(match_health(7200), MatchHealth::Critical); // exactly 1 day
}

#[test]
fn test_match_health_warning_boundaries() {
    assert_eq!(match_health(7201), MatchHealth::Warning);
    assert_eq!(match_health(25200), MatchHealth::Warning); // ~3.5 days
    assert_eq!(match_health(50400), MatchHealth::Warning); // exactly 7 days
}

#[test]
fn test_match_health_healthy() {
    assert_eq!(match_health(50401), MatchHealth::Healthy);
    assert_eq!(match_health(1_000_000), MatchHealth::Healthy);
}

// ---------------------------------------------------------------------------
// compute_match_deadline — full struct
// ---------------------------------------------------------------------------

#[test]
fn test_compute_match_deadline_healthy() {
    let match_info = make_match(1000, 30_000_000_000, 0, 1000);
    let deadline = compute_match_deadline(&match_info, 1000);
    assert_eq!(deadline.health, MatchHealth::Healthy);
    assert!(!deadline.remaining_capacity_ckb.is_nan());
    assert!(deadline.blocks_remaining > 0);
}

#[test]
fn test_compute_match_deadline_exhausted() {
    // Capacity 1000 shannons, rate 100 per block, last extraction at block 0,
    // created at block 0, tip at block 20.
    // Accumulated rent = 100 * 20 = 2000, remaining = 1000 → exhausted
    let match_info = make_match(100, 1000, 0, 0);
    let deadline = compute_match_deadline(&match_info, 20);
    assert_eq!(deadline.health, MatchHealth::Exhausted);
    assert_eq!(deadline.blocks_remaining, 0);
    assert!(deadline.extractable_now_ckb > 0.0);
}

#[test]
fn test_compute_match_deadline_last_extraction() {
    // Match created at 1000, extracted at 5000, tip at 6000
    // rate=1000, remaining=5_000_000 shannons
    let match_info = make_match(1000, 5_000_000, 5000, 1000);
    let deadline = compute_match_deadline(&match_info, 6000);
    // Extractable = 1000 * (6000 - 5000) = 1_000_000
    // blocks_remaining = ceil(5_000_000 / 1000) = 5000 from baseline 5000, so exhaustion at 10000
    // blocks_remaining = 10000 - 6000 = 4000
    assert_eq!(deadline.last_extraction_block, 5000);
    assert_eq!(deadline.match_creation_block, 1000);
    assert_eq!(deadline.projected_exhaustion_block, 10000);
    assert_eq!(deadline.blocks_remaining, 4000);
    assert!(deadline.extractable_now_ckb > 0.0);
}

// ---------------------------------------------------------------------------
// sort_by_urgency
// ---------------------------------------------------------------------------

#[test]
fn test_sort_by_urgency_empty() {
    use opticrum_sdk::deadline::sort_by_urgency;
    let mut deadlines: Vec<opticrum_sdk::types::MatchDeadline> = vec![];
    sort_by_urgency(&mut deadlines);
    assert!(deadlines.is_empty());
}

#[test]
fn test_sort_by_urgency_ordering() {
    use opticrum_sdk::deadline::sort_by_urgency;
    use opticrum_sdk::types::MatchDeadline;

    let make = |blocks_remaining: u64| -> MatchDeadline {
        MatchDeadline {
            match_outpoint: format!("tx:{}", blocks_remaining),
            channel_outpoint: "ch:0".into(),
            shannons_per_block: 1000,
            remaining_capacity_ckb: 1.0,
            last_extraction_block: 0,
            match_creation_block: 0,
            projected_exhaustion_block: blocks_remaining,
            blocks_remaining,
            estimated_hours_remaining: blocks_remaining as f64 / 300.0,
            health: opticrum_sdk::types::MatchHealth::Healthy,
            extractable_now_ckb: 0.0,
        }
    };

    // Unsorted: 100, 0 (exhausted), 5000, 1
    let mut deadlines = vec![make(100), make(0), make(5000), make(1)];
    sort_by_urgency(&mut deadlines);

    // Should be sorted ascending by blocks_remaining
    assert_eq!(deadlines[0].blocks_remaining, 0);
    assert_eq!(deadlines[1].blocks_remaining, 1);
    assert_eq!(deadlines[2].blocks_remaining, 100);
    assert_eq!(deadlines[3].blocks_remaining, 5000);
}

// ---------------------------------------------------------------------------
// find_exhausted_matches — integration test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_find_exhausted_matches_with_data() {
    let mut rpc = test_rpc().await;

    // Seed a match that is already exhausted at tip=20
    // capacity = 1000, rate = 100/block, created at 0
    let order_args = test_order_args();
    let ch_op = channel_outpoint();
    let match_args = MatchArgs::new(order_args, ch_op, seller_lock_hash());
    let match_data = MatchData::new(0, 100);
    seed_match(&mut rpc, &match_args, &match_data, 1000, 0).await;

    // Seed another match that is NOT exhausted
    // capacity = 30 CKB = 3_000_000_000, rate = 1000/block → very far from exhausted
    let order_args2 = test_order_args();
    let ch_op2 = channel_outpoint();
    let match_args2 = MatchArgs::new(order_args2, ch_op2, seller_lock_hash());
    let match_data2 = MatchData::new(0, 1000);
    seed_match(
        &mut rpc,
        &match_args2,
        &match_data2,
        RENT_CAPACITY,
        MATCH_CREATED_BLOCK,
    )
    .await;

    // Find exhausted matches (tip at block 20)
    let exhausted = find_exhausted_matches(&rpc, 20, None).await.unwrap();
    // The first match (capacity=1000, rate=100, created=0) should be exhausted at tip=20
    assert!(
        !exhausted.is_empty(),
        "should find at least one exhausted match"
    );
    for d in &exhausted {
        assert_eq!(d.health, MatchHealth::Exhausted);
        assert_eq!(d.blocks_remaining, 0);
    }
}

// ---------------------------------------------------------------------------
// find_matches_near_exhaustion — integration test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_find_matches_near_exhaustion_with_data() {
    let mut rpc = test_rpc().await;

    // Seed a match: capacity=100, rate=10/block, created at 0
    // blocks to exhaust = ceil(100/10) = 10 from baseline 0 → 10
    // At tip=5: blocks_remaining = 10 - 5 = 5 (< 100 threshold)
    let order_args = test_order_args();
    let ch_op = channel_outpoint();
    let match_args = MatchArgs::new(order_args, ch_op, seller_lock_hash());
    let match_data = MatchData::new(0, 10);
    seed_match(&mut rpc, &match_args, &match_data, 100, 0).await;

    // Seed a match far from exhaustion
    let order_args2 = test_order_args();
    let ch_op2 = channel_outpoint();
    let match_args2 = MatchArgs::new(order_args2, ch_op2, seller_lock_hash());
    let match_data2 = MatchData::new(0, 1000);
    seed_match(
        &mut rpc,
        &match_args2,
        &match_data2,
        RENT_CAPACITY,
        MATCH_CREATED_BLOCK,
    )
    .await;

    // Find matches within 100 blocks of exhaustion at tip=5
    let near = find_matches_near_exhaustion(&rpc, 5, 100, None)
        .await
        .unwrap();
    assert!(
        !near.is_empty(),
        "should find at least one near-exhaustion match"
    );

    // With a very tight threshold, only the close-to-exhaustion match should appear
    let tight = find_matches_near_exhaustion(&rpc, 5, 1, None)
        .await
        .unwrap();
    // All results should have blocks_remaining <= 1
    for d in &tight {
        assert!(
            d.blocks_remaining <= 1,
            "tight threshold: blocks_remaining {} should be <= 1",
            d.blocks_remaining
        );
    }
}
