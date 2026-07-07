//! Full lifecycle tests — verify the SDK can build valid skeletons for each
//! stage of the Opticrum state machine.

mod common;
use common::*;
use opticrum_calculator::types::{MatchArgs, MatchData, MatchInfo};
use opticrum_sdk::sdk::OpticrumSdk;

// ---------------------------------------------------------------------------
// Lifecycle: Create order (build and verify skeleton structure)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_lifecycle_create_order_full() {
    let mut rpc = test_rpc().await;
    seed_user_cell(&mut rpc, 200_000_000_000);

    let sdk = OpticrumSdk::new(rpc);
    let buyer = test_address();
    let order_args = test_order_args();
    let order_data = test_order_data();

    // Build create-order skeleton with fiber address
    let skel = sdk
        .build_create_order(
            buyer,
            &order_args,
            &order_data,
            RENT_CAPACITY,
            None,
            Some("/ip4/1.2.3.4/tcp/9735".to_string()),
        )
        .await
        .unwrap();

    // Verify skeleton structure
    assert!(!skel.inputs.is_empty(), "should have buyer input");
    assert!(!skel.outputs.is_empty(), "should have order output");
    assert_eq!(skel.outputs[0].data.len(), 32, "order data = 32 bytes");

    // Verify the witness was set (fiber_address in output_type)
    let witness_index = skel.inputs.len(); // input_count + output_index (0)
    assert!(
        witness_index < skel.witnesses.len(),
        "witness slot should exist for fiber_address"
    );
    assert!(!skel.witnesses[witness_index].empty);
    assert_eq!(
        skel.witnesses[witness_index].output_type,
        b"/ip4/1.2.3.4/tcp/9735"
    );
}

// ---------------------------------------------------------------------------
// Lifecycle: Cancel order (build and verify)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_lifecycle_cancel_order() {
    let mut rpc = test_rpc().await;
    seed_user_cell(&mut rpc, 200_000_000_000);

    let order_args = test_order_args();
    let order_data = test_order_data();
    let order_info = seed_order(&mut rpc, &order_args, &order_data, RENT_CAPACITY).await;

    let sdk = OpticrumSdk::new(rpc);
    let skel = sdk
        .build_cancel_order(test_address(), order_info)
        .await
        .unwrap();

    // Cancel = Burn: order cell consumed as input
    assert!(
        skel.inputs.len() >= 2,
        "should have order input + buyer input"
    );
}

// ---------------------------------------------------------------------------
// Lifecycle: Match (build and verify)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_lifecycle_match_order() {
    let mut rpc = test_rpc().await;
    seed_user_cell(&mut rpc, 200_000_000_000);
    seed_seller_cell(&mut rpc, 200_000_000_000);
    let ch_op = channel_outpoint();
    seed_channel_cell(&mut rpc, &ch_op, CHANNEL_CAPACITY);

    let order_args = test_order_args();
    let order_data = test_order_data();
    let order_info = seed_order(&mut rpc, &order_args, &order_data, RENT_CAPACITY).await;

    let match_args = MatchArgs::new(order_args, ch_op, seller_lock_hash());

    let sdk = OpticrumSdk::new(rpc);
    let skel = sdk
        .build_match_order(seller_address(), order_info, match_args)
        .await
        .unwrap();

    assert!(!skel.inputs.is_empty(), "should have inputs");
    assert!(!skel.outputs.is_empty(), "should have match output");
    assert_eq!(skel.outputs[0].data.len(), 32, "match data = 32 bytes");
}

// ---------------------------------------------------------------------------
// Lifecycle: Extract (not exhausted)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_lifecycle_extract_rent() {
    let mut rpc = test_rpc().await;
    seed_seller_cell(&mut rpc, 200_000_000_000);
    let ch_op = channel_outpoint();
    seed_channel_cell(&mut rpc, &ch_op, CHANNEL_CAPACITY);

    let order_args = test_order_args();
    let match_args = MatchArgs::new(order_args, ch_op, seller_lock_hash());
    let match_data = MatchData::new(0, SHANNONS_PER_BLOCK);

    // Seed a real match cell so the SDK can find it by outpoint
    let match_info = seed_match(
        &mut rpc,
        &match_args,
        &match_data,
        RENT_CAPACITY,
        MATCH_CREATED_BLOCK,
    )
    .await;

    // Seed header at tip_block for AddHeaderDepByBlockNumber
    let tip_block = MATCH_CREATED_BLOCK + 100;
    seed_header(&mut rpc, tip_block, 1_000_000);

    let sdk = OpticrumSdk::new(rpc);
    let skel = sdk
        .build_extract_rent(seller_address(), match_info, tip_block)
        .await
        .expect("extract_rent should succeed for a non-exhausted match");

    assert!(
        !skel.inputs.is_empty(),
        "extract: should have match + seller inputs"
    );
    assert!(
        skel.inputs.len() >= 2,
        "extract: expected at least 2 inputs"
    );
    assert!(
        !skel.outputs.is_empty(),
        "extract: should have updated match output"
    );
}

// ---------------------------------------------------------------------------
// Lifecycle: Exhaustion guard prevents premature destroy
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_lifecycle_guards() {
    let mut rpc = test_rpc().await;
    seed_seller_cell(&mut rpc, 200_000_000_000);

    let ch_op = channel_outpoint();
    let order_args = test_order_args();
    let match_args = MatchArgs::new(order_args, ch_op, seller_lock_hash());

    // Fresh match (far from exhausted)
    let match_info = MatchInfo {
        match_args,
        match_data: MatchData::new(0, SHANNONS_PER_BLOCK),
        xudt: None,
        ckb_capacity: RENT_CAPACITY,
        match_outpoint: channel_outpoint(),
        match_current_block: MATCH_CREATED_BLOCK,
    };

    let sdk = OpticrumSdk::new(rpc);

    // Destroy should be rejected — not exhausted
    let result = sdk
        .build_destroy_match(seller_address(), match_info, MATCH_CREATED_BLOCK)
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not exhausted"));
}

// ---------------------------------------------------------------------------
// Lifecycle: Update match (buyer injects/withdraws capacity)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_lifecycle_update_match() {
    let mut rpc = test_rpc().await;
    seed_user_cell(&mut rpc, 200_000_000_000);
    let ch_op = channel_outpoint();
    seed_channel_cell(&mut rpc, &ch_op, CHANNEL_CAPACITY);

    let order_args = test_order_args();
    let match_args = MatchArgs::new(order_args, ch_op, seller_lock_hash());
    let match_data = MatchData::new(0, SHANNONS_PER_BLOCK);

    // Seed a real match cell so the SDK can find it by outpoint
    let match_info = seed_match(
        &mut rpc,
        &match_args,
        &match_data,
        RENT_CAPACITY,
        MATCH_CREATED_BLOCK,
    )
    .await;

    let sdk = OpticrumSdk::new(rpc);
    // Inject 100 CKB (positive delta)
    let skel = sdk
        .build_update_match(test_address(), match_info, 0, 10_000_000_000)
        .await
        .expect("update_match should succeed");

    assert!(
        !skel.inputs.is_empty(),
        "update: should have match + buyer inputs"
    );
    assert!(skel.inputs.len() >= 2, "update: expected at least 2 inputs");
    assert!(
        !skel.outputs.is_empty(),
        "update: should have updated match output"
    );
}

// ---------------------------------------------------------------------------
// Lifecycle: Destroy exhausted match (success path)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_lifecycle_destroy_match_exhausted() {
    let mut rpc = test_rpc().await;
    seed_seller_cell(&mut rpc, 200_000_000_000);
    let ch_op = channel_outpoint();
    seed_channel_cell(&mut rpc, &ch_op, CHANNEL_CAPACITY);

    let order_args = test_order_args();
    let match_args = MatchArgs::new(order_args, ch_op, seller_lock_hash());

    // Seed a match with capacity=1000, rate=100/block, created at 0.
    // At tip=20: accumulated = 100*20 = 2000 > 1000 → exhausted.
    let match_data = MatchData::new(0, 100);
    let match_creation_block: u64 = 0;
    let match_info = seed_match(
        &mut rpc,
        &match_args,
        &match_data,
        1000, // small capacity so it exhausts quickly
        match_creation_block,
    )
    .await;

    // Seed headers needed by destroy_match:
    //   [0] tip block (20) for exhaustion verification
    //   [1] match creation block (0) for baseline computation
    let tip_block: u64 = 20;
    seed_header(&mut rpc, tip_block, 1_000_000);
    seed_header(&mut rpc, match_creation_block, 1_000_000);

    let sdk = OpticrumSdk::new(rpc);
    let skel = sdk
        .build_destroy_match(seller_address(), match_info, tip_block)
        .await
        .expect("destroy_match should succeed for an exhausted match");

    assert!(
        !skel.inputs.is_empty(),
        "destroy exhausted: should have match + seller inputs"
    );
    assert!(
        skel.inputs.len() >= 2,
        "destroy exhausted: expected at least 2 inputs"
    );
    assert!(
        !skel.outputs.is_empty(),
        "destroy exhausted: should have seller output"
    );
}
