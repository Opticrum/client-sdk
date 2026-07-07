//! Error path tests — verify SDK error handling.

mod common;
use common::*;
use opticrum_calculator::types::{MatchArgs, MatchData, MatchInfo};
use opticrum_sdk::{error::SdkError, sdk::OpticrumSdk};

// ---------------------------------------------------------------------------
// build_destroy_match — NotExhausted guard
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_destroy_not_exhausted_error() {
    let mut rpc = test_rpc().await;
    seed_user_cell(&mut rpc, 200_000_000_000);
    seed_seller_cell(&mut rpc, 200_000_000_000);

    // Build a match that is far from exhausted
    let order_args = test_order_args();
    let ch_op = channel_outpoint();
    let match_args = MatchArgs::new(order_args, ch_op, seller_lock_hash());
    // Large capacity, small rate → not exhausted
    let match_data = MatchData::new(0, 100);

    let match_info = MatchInfo {
        match_args,
        match_data,
        xudt: None,
        ckb_capacity: 100_000_000_000, // 1000 CKB
        match_outpoint: channel_outpoint(),
        match_current_block: MATCH_CREATED_BLOCK,
    };

    let sdk = OpticrumSdk::new(rpc);
    let result = sdk
        .build_destroy_match(seller_address(), match_info, MATCH_CREATED_BLOCK)
        .await;

    assert!(result.is_err());
    match result.unwrap_err() {
        SdkError::NotExhausted(remaining) => {
            assert!(remaining > 0.0);
        }
        other => panic!("expected NotExhausted, got {other}"),
    }
}

// ---------------------------------------------------------------------------
// build_destroy_match — not authorized (wrong caller)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_sdk_does_not_enforce_auth() {
    // Verify the SDK does NOT enforce seller-only authorization.
    // Authorization is checked by the on-chain verifier, not the SDK.
    let mut rpc = test_rpc().await;
    seed_user_cell(&mut rpc, 200_000_000_000);
    let ch_op = channel_outpoint();
    seed_channel_cell(&mut rpc, &ch_op, CHANNEL_CAPACITY);

    let order_args = test_order_args();
    let match_args = MatchArgs::new(order_args, ch_op, seller_lock_hash());

    // Seed an exhausted match: capacity=1000, rate=100/block, created at 0.
    // At tip=20: accumulated rent = 100×20 = 2000 > 1000 → exhausted.
    let match_data = MatchData::new(0, 100);
    let match_creation_block: u64 = 0;
    let match_info = seed_match(
        &mut rpc,
        &match_args,
        &match_data,
        1000,
        match_creation_block,
    )
    .await;

    // Seed headers destroy_match needs:
    //   HeaderDep[0] = tip block for exhaustion verification
    //   HeaderDep[1] = match creation block (last_extraction_block==0 case)
    let tip_block: u64 = 20;
    seed_header(&mut rpc, tip_block, 1_000_000);
    seed_header(&mut rpc, match_creation_block, 1_000_000);

    let sdk = OpticrumSdk::new(rpc);
    // Call destroy with test_address() (BUYER, not seller).
    // The SDK must pass the exhaustion guard AND proceed to build the tx.
    // If the SDK had an auth check, this would fail with NotAuthorized.
    let skel = sdk
        .build_destroy_match(test_address(), match_info, tip_block)
        .await
        .expect("SDK should not reject non-seller; auth is enforced on-chain");

    assert!(!skel.inputs.is_empty(), "should have match + buyer inputs");
    assert!(skel.inputs.len() >= 2, "expected at least 2 inputs");
    assert!(!skel.outputs.is_empty(), "should have output");
}

// ---------------------------------------------------------------------------
// SdkError Display
// ---------------------------------------------------------------------------

#[test]
fn test_sdk_error_display() {
    let err = SdkError::Chain("test".into());
    assert!(err.to_string().contains("chain error"));

    let err = SdkError::Scan("test".into());
    assert!(err.to_string().contains("scan error"));

    let err = SdkError::Build("test".into());
    assert!(err.to_string().contains("build error"));

    let err = SdkError::InvalidInput("bad".into());
    assert!(err.to_string().contains("invalid input"));

    let err = SdkError::AlreadyExhausted(42);
    assert!(err.to_string().contains("42"));

    let err = SdkError::NotExhausted(1.5);
    assert!(err.to_string().contains("1.5"));

    let err = SdkError::NotAuthorized("nope".into());
    assert!(err.to_string().contains("not authorized"));
}

#[test]
fn test_sdk_error_trait() {
    // Verify SdkError implements std::error::Error
    use std::error::Error;
    let err: Box<dyn Error> = Box::new(SdkError::Chain("test".into()));
    // .source() should return None (SdkError doesn't wrap other errors)
    assert!(err.source().is_none());

    // Verify Display via the Error trait
    assert!(err.to_string().contains("chain error"));
}
