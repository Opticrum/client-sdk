//! Integration tests for the opticrum-sdk core functions.
//!
//! Uses `FakeRpcClient` for deterministic, in-memory chain state.
//! No CKB node or compiled contract binary required.

mod common;

use common::*;
use opticrum_calculator::types::{CompressedPubkey, MatchArgs, MatchData, MatchInfo};
use opticrum_sdk::sdk::OpticrumSdk;

// ---------------------------------------------------------------------------
// Read operations
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_tip_block() {
    let rpc = test_rpc().await;
    let sdk = OpticrumSdk::new(rpc);
    // FakeRpcClient defaults to tip = 0
    let tip = sdk.get_tip_block().await.unwrap();
    assert_eq!(tip, 0);
}

#[tokio::test]
async fn test_scan_orders_empty() {
    let rpc = test_rpc().await;
    let sdk = OpticrumSdk::new(rpc);
    let orders = sdk.scan_orders(None).await.unwrap();
    assert!(orders.is_empty());
}

#[tokio::test]
async fn test_scan_orders_with_data() {
    let mut rpc = test_rpc().await;
    let order_args = test_order_args();
    let order_data = test_order_data();

    // Seed an order cell
    let seeded = seed_order(&mut rpc, &order_args, &order_data, RENT_CAPACITY).await;

    let sdk = OpticrumSdk::new(rpc);
    let orders = sdk.scan_orders(None).await.unwrap();
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].order_args, seeded.order_args);
    assert_eq!(orders[0].order_data, seeded.order_data);
}

#[tokio::test]
async fn test_scan_matches_empty() {
    let rpc = test_rpc().await;
    let sdk = OpticrumSdk::new(rpc);
    let matches = sdk.scan_matches(None).await.unwrap();
    assert!(matches.is_empty());
}

// ---------------------------------------------------------------------------
// build_create_order
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_build_create_order_skeleton() {
    let mut rpc = test_rpc().await;
    // Seed a user cell with enough capacity
    seed_user_cell(&mut rpc, 200_000_000_000);

    let sdk = OpticrumSdk::new(rpc);
    let buyer = test_address();
    let order_args = test_order_args();
    let order_data = test_order_data();

    let skeleton = sdk
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

    // Verify the skeleton has expected structure
    // There should be at least 1 input (buyer cell) and 1 output (order cell)
    assert!(!skeleton.inputs.is_empty(), "should have buyer input");
    assert!(!skeleton.outputs.is_empty(), "should have order output");

    // The output should have our data
    let output_data = &skeleton.outputs[0].data;
    assert_eq!(output_data.len(), 32, "order data should be 32 bytes");
}

#[tokio::test]
async fn test_build_create_order_no_fiber_address() {
    let mut rpc = test_rpc().await;
    seed_user_cell(&mut rpc, 200_000_000_000);

    let sdk = OpticrumSdk::new(rpc);
    let buyer = test_address();
    let order_args = test_order_args();
    let order_data = test_order_data();

    let skeleton = sdk
        .build_create_order(buyer, &order_args, &order_data, RENT_CAPACITY, None, None)
        .await
        .unwrap();

    assert!(!skeleton.inputs.is_empty());
    assert!(!skeleton.outputs.is_empty());
}

// ---------------------------------------------------------------------------
// build_cancel_order
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_build_cancel_order_skeleton() {
    let mut rpc = test_rpc().await;
    seed_user_cell(&mut rpc, 200_000_000_000);

    let order_args = test_order_args();
    let order_data = test_order_data();
    let order_info = seed_order(&mut rpc, &order_args, &order_data, RENT_CAPACITY).await;

    let sdk = OpticrumSdk::new(rpc);
    let skeleton = sdk
        .build_cancel_order(test_address(), order_info)
        .await
        .unwrap();

    // Cancel = Burn pattern: the order cell is consumed as input
    assert!(
        skeleton.inputs.len() >= 2,
        "should have order input + buyer input"
    );
}

// ---------------------------------------------------------------------------
// build_destroy_match — guards
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_build_destroy_match_not_exhausted() {
    let mut rpc = test_rpc().await;
    seed_user_cell(&mut rpc, 200_000_000_000);
    seed_seller_cell(&mut rpc, 200_000_000_000);

    let order_args = test_order_args();
    let _order_data = test_order_data();
    let ch_outpoint = channel_outpoint();
    seed_channel_cell(&mut rpc, &ch_outpoint, CHANNEL_CAPACITY);

    let match_args = test_match_args(&order_args, &ch_outpoint);
    let match_data = test_match_data();

    // Build a match_info where shannons_per_block = 1000, capacity = 30 CKB,
    // match created at MATCH_CREATED_BLOCK (1000), tip = 1000
    let match_info = MatchInfo {
        match_args,
        match_data,
        xudt: None,
        ckb_capacity: RENT_CAPACITY,
        match_outpoint: channel_outpoint(),
        match_current_block: MATCH_CREATED_BLOCK,
    };

    let sdk = OpticrumSdk::new(rpc);
    // Tip is close to match creation → not exhausted
    let result = sdk
        .build_destroy_match(seller_address(), match_info, MATCH_CREATED_BLOCK)
        .await;

    assert!(result.is_err(), "should reject non-exhausted destroy");
    match result.unwrap_err() {
        opticrum_sdk::error::SdkError::NotExhausted(_) => {} // expected
        other => panic!("expected NotExhausted, got {other}"),
    }
}

// ---------------------------------------------------------------------------
// build_update_match
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_build_update_match_positive_delta() {
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
    // Inject capacity: positive delta (100 CKB)
    let skel = sdk
        .build_update_match(test_address(), match_info, 0, 10_000_000_000)
        .await
        .expect("build_update_match with positive delta should succeed");

    assert!(
        !skel.inputs.is_empty(),
        "should have match input + buyer input"
    );
    assert!(
        skel.inputs.len() >= 2,
        "expected at least 2 inputs (match + buyer)"
    );
    assert!(!skel.outputs.is_empty(), "should have updated match output");
}

#[tokio::test]
async fn test_build_update_match_negative_delta() {
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
    // Withdraw capacity: negative delta (50 CKB)
    let skel = sdk
        .build_update_match(test_address(), match_info, 0, -5_000_000_000)
        .await
        .expect("build_update_match with negative delta should succeed");

    assert!(
        !skel.inputs.is_empty(),
        "should have match input + buyer input"
    );
    assert!(
        skel.inputs.len() >= 2,
        "expected at least 2 inputs (match + buyer)"
    );
    assert!(!skel.outputs.is_empty(), "should have updated match output");
}

// ---------------------------------------------------------------------------
// scan_orders with pubkey filter
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_scan_orders_with_pubkey_filter() {
    let mut rpc = test_rpc().await;
    let order_args = test_order_args();
    let order_data = test_order_data();
    seed_order(&mut rpc, &order_args, &order_data, RENT_CAPACITY).await;

    let sdk = OpticrumSdk::new(rpc);

    // Filter by the matching pubkey — should find the order
    let matching_pk = fiber_pubkey();
    let orders = sdk.scan_orders(Some(matching_pk)).await.unwrap();
    assert_eq!(orders.len(), 1);

    // Filter by a different pubkey — should return empty
    let other_pk = CompressedPubkey::new([0x03u8; 33]);
    let orders = sdk.scan_orders(Some(other_pk)).await.unwrap();
    assert!(orders.is_empty());
}

// ---------------------------------------------------------------------------
// scan_matches with data
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_scan_matches_with_data() {
    let mut rpc = test_rpc().await;
    let order_args = test_order_args();
    let ch_op = channel_outpoint();
    let match_args = test_match_args(&order_args, &ch_op);
    let match_data = test_match_data();

    // Seed a match cell
    seed_match(
        &mut rpc,
        &match_args,
        &match_data,
        RENT_CAPACITY,
        MATCH_CREATED_BLOCK,
    )
    .await;

    let sdk = OpticrumSdk::new(rpc);
    let matches = sdk.scan_matches(None).await.unwrap();
    assert!(!matches.is_empty(), "should find at least one match");
    // Verify structural data — the scan found match cells with correct data
    let target = matches
        .iter()
        .find(|m| m.match_data.shannons_per_block == SHANNONS_PER_BLOCK)
        .expect("should find the match with our shannons_per_block");
    assert_eq!(target.match_data.shannons_per_block, SHANNONS_PER_BLOCK);
}

// ---------------------------------------------------------------------------
// build_create_order with xUDT type script
// ---------------------------------------------------------------------------

#[tokio::test]
#[allow(clippy::useless_vec)]
async fn test_build_create_order_with_xudt() {
    let mut rpc = test_rpc().await;
    seed_user_cell(&mut rpc, 200_000_000_000);

    // Seed the xUDT contract celldep so AddXudtCelldep can find it.
    // The xUDT contract on fakenet is at a random tx_hash (lazy_static).
    // We seed a dummy cell there — its content doesn't matter for a celldep.
    {
        let xudt_tx_hash =
            ckb_cinnabar_calculator::operation::udt::hardcoded::XUDT_FAKENET_TX_HASH.clone();
        let xudt_outpoint = {
            use ckb_cinnabar_calculator::re_exports::ckb_types::{
                packed::OutPoint,
                prelude::{Builder, Entity, Pack},
            };
            OutPoint::new_builder()
                .tx_hash(xudt_tx_hash.pack())
                .index(0u32.pack())
                .build()
        };
        let dummy_cell = ckb_cinnabar_calculator::skeleton::CellOutputEx::new_from_scripts(
            ckb_cinnabar_calculator::simulation::always_success_script(vec![]),
            None,
            vec![],
            Some(
                ckb_cinnabar_calculator::re_exports::ckb_types::core::Capacity::shannons(
                    100_000_000,
                ),
            ),
        )
        .expect("build dummy xudt celldep cell");
        rpc.insert_fake_cell(
            xudt_outpoint,
            dummy_cell,
            Some(ckb_cinnabar_calculator::simulation::fake_header_view(
                0, 0, 0,
            )),
        );
    }

    // Build a simple xUDT type script
    let xudt_type_script = {
        use ckb_cinnabar_calculator::re_exports::ckb_types::{
            core::ScriptHashType,
            packed::Script,
            prelude::{Builder, Entity, Pack},
            H256,
        };
        Script::new_builder()
            .code_hash(H256([0xABu8; 32]).pack())
            .hash_type(ScriptHashType::Type.into())
            .args(vec![0x00u8; 32].pack())
            .build()
    };

    let sdk = OpticrumSdk::new(rpc);
    let buyer = test_address();
    let order_args = test_order_args();
    let order_data = test_order_data();

    let skel = sdk
        .build_create_order(
            buyer,
            &order_args,
            &order_data,
            RENT_CAPACITY,
            Some(xudt_type_script),
            None,
        )
        .await
        .expect("build_create_order with xUDT should succeed");

    assert!(!skel.inputs.is_empty(), "should have buyer input");
    assert!(!skel.outputs.is_empty(), "should have order output");
    assert!(
        skel.celldeps.len() >= 2,
        "should have opticrum + xUDT celldeps, got {}",
        skel.celldeps.len()
    );
}

// ---------------------------------------------------------------------------
// scan_orders: multiple orders
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_scan_orders_multiple() {
    let mut rpc = test_rpc().await;
    let order_args = test_order_args();
    let order_data = test_order_data();

    // Seed 3 orders
    seed_order(&mut rpc, &order_args, &order_data, RENT_CAPACITY).await;
    seed_order(&mut rpc, &order_args, &order_data, RENT_CAPACITY).await;
    seed_order(&mut rpc, &order_args, &order_data, RENT_CAPACITY).await;

    let sdk = OpticrumSdk::new(rpc);
    let orders = sdk.scan_orders(None).await.unwrap();
    assert_eq!(orders.len(), 3);
}
