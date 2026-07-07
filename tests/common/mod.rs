//! Shared test helpers for opticrum-sdk tests.
//!
//! Uses `FakeRpcClient` from ckb-cinnabar to provide deterministic,
//! in-memory chain state. No CKB node required.

// Each test binary uses a different subset of helpers — suppress per-file dead_code.
#![allow(dead_code, unused_imports)]

use ckb_cinnabar_calculator::{
    address::{Address, AddressPayload},
    instruction::{Instruction, TransactionCalculator},
    re_exports::ckb_types::{
        bytes::Bytes,
        core::{Capacity, ScriptHashType},
        packed::{OutPoint as PackedOutPoint, Script},
        prelude::{Builder, Entity, Pack, Unpack},
        H256,
    },
    simulation::{
        always_success_script, fake_header_view, fake_outpoint, AddFakeAlwaysSuccessCelldep,
        AddFakeContractCelldepByName, FakeRpcClient,
    },
    skeleton::CellOutputEx,
};
use opticrum_calculator::types::{
    CompressedPubkey, MatchArgs, MatchData, MatchInfo, OrderArgs, OrderData, OrderInfo, OutPoint,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const CHANNEL_CAPACITY: u64 = 100_000_000_000; // 1000 CKB
pub const SHANNONS_PER_BLOCK: u64 = 1000;
pub const RENT_CAPACITY: u64 = 30_000_000_000;
pub const ORDER_CREATED_BLOCK: u64 = 10;
pub const CHANNEL_CREATED_BLOCK: u64 = 20;
pub const MATCH_CREATED_BLOCK: u64 = 1000;

/// Hardcoded mock for the Fiber funding type script.
pub const CONTRACT_MOCK: [u8; 32] = [
    0x77, 0xc9, 0x16, 0x3a, 0xdd, 0xbf, 0x87, 0xc8, 0x05, 0xbe, 0x3b, 0x6c, 0x85, 0x69, 0xb8, 0xe0,
    0x15, 0xa4, 0xca, 0x0e, 0xf3, 0xc6, 0x89, 0x15, 0x02, 0x34, 0xf0, 0xc8, 0x02, 0xa7, 0x69, 0x00,
];

// ---------------------------------------------------------------------------
// RPC + Address helpers
// ---------------------------------------------------------------------------

/// Create a `FakeRpcClient` pre-loaded with always-success and Opticrum
/// contract cell deps, ready for SDK operations.
pub async fn test_rpc() -> FakeRpcClient {
    let mut rpc = FakeRpcClient::default();
    // Seed contract into the skeleton (for always-success dep)
    let _skeleton = celldeps_prepared_skeleton(&rpc).await.unwrap();
    // Also seed the contract directly into the RPC so the SDK can find it
    // when it creates fresh skeletons
    seed_opticrum_contract_into_rpc(&mut rpc);
    rpc
}

/// Create a test address using the always-success lock (blank args = buyer).
pub fn test_address() -> Address {
    let lock = always_success_script(vec![]);
    let payload = AddressPayload::from(lock);
    Address::new(ckb_cinnabar_calculator::rpc::Network::Fake, payload)
}

/// Create a distinct seller address with different lock args.
pub fn seller_address() -> Address {
    let lock = always_success_script(vec![0x01]);
    let payload = AddressPayload::from(lock);
    Address::new(ckb_cinnabar_calculator::rpc::Network::Fake, payload)
}

// ---------------------------------------------------------------------------
// Test data builders
// ---------------------------------------------------------------------------

pub fn test_order_args() -> OrderArgs {
    OrderArgs::new(fiber_pubkey(), user_lock_hash())
}

pub fn test_order_data() -> OrderData {
    OrderData::new(0, CHANNEL_CAPACITY, SHANNONS_PER_BLOCK)
}

pub fn test_match_args(order_args: &OrderArgs, ch_outpoint: &OutPoint) -> MatchArgs {
    MatchArgs::new(order_args.clone(), ch_outpoint.clone(), seller_lock_hash())
}

pub fn test_match_data() -> MatchData {
    MatchData::new(0, SHANNONS_PER_BLOCK)
}

pub fn fiber_pubkey() -> CompressedPubkey {
    CompressedPubkey::new([0x02u8; 33])
}

pub fn user_lock_hash() -> [u8; 32] {
    let mut hash = [0u8; 32];
    let script_hash = ckb_cinnabar_calculator::re_exports::ckb_hash::blake2b_256(
        always_success_script(vec![]).as_slice(),
    );
    hash.copy_from_slice(&script_hash);
    hash
}

pub fn seller_lock_hash() -> [u8; 32] {
    let mut hash = [0u8; 32];
    let script_hash = ckb_cinnabar_calculator::re_exports::ckb_hash::blake2b_256(
        always_success_script(vec![0x01]).as_slice(),
    );
    hash.copy_from_slice(&script_hash);
    hash
}

pub fn channel_outpoint() -> OutPoint {
    let op = fake_outpoint();
    OutPoint::new(op.tx_hash().unpack(), op.index().unpack())
}

pub fn random_u64() -> u64 {
    use ckb_cinnabar_calculator::simulation::random_hash;
    let hash = random_hash();
    u64::from_le_bytes(hash[..8].try_into().unwrap())
}

// ---------------------------------------------------------------------------
// Skeleton preparation
// ---------------------------------------------------------------------------

/// Build a reusable skeleton pre-loaded with the Opticrum contract and
/// always-success celldeps.
async fn celldeps_prepared_skeleton(
    rpc: &FakeRpcClient,
) -> Result<ckb_cinnabar_calculator::skeleton::TransactionSkeleton, String> {
    let prepare = Instruction::<FakeRpcClient>::new(vec![
        Box::new(AddFakeAlwaysSuccessCelldep {}),
        Box::new(AddFakeContractCelldepByName {
            contract: "opticrum".to_string(),
            type_id_args: Some(H256::default()),
            contract_binary_path: "../opticrum/build/release".to_string(),
        }),
    ]);
    let (skeleton, _) = TransactionCalculator::default()
        .instruction(prepare)
        .new_skeleton(rpc)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(skeleton)
}

/// Seed the Opticrum contract cell directly into the FakeRpcClient's fake
/// cells so that `AddCellDepByType` can find it via `GetCellsIter`.
/// This is needed because the SDK creates a fresh skeleton for each call,
/// unlike the integration tests which reuse a single skeleton.
fn seed_opticrum_contract_into_rpc(rpc: &mut FakeRpcClient) {
    use ckb_cinnabar_calculator::re_exports::ckb_types::core::ScriptHashType;
    use std::fs;

    let contract_path = std::path::PathBuf::from("../opticrum/build/release/opticrum");
    let contract_data = fs::read(&contract_path)
        .unwrap_or_else(|e| panic!("Failed to read contract binary at {:?}: {e}", contract_path));

    // Run AddFakeContractCelldep which both adds to skeleton AND seeds into RPC.
    // But actually, AddFakeContractCelldep only adds to skeleton. We need to
    // manually seed the cell into fake_rpc.
    let type_script = Script::new_builder()
        .code_hash(H256(ckb_cinnabar_calculator::skeleton::TYPE_ID_CODE_HASH.into()).pack())
        .hash_type(ScriptHashType::Type.into())
        .args(H256::default().as_bytes().pack())
        .build();

    let output = ckb_cinnabar_calculator::re_exports::ckb_types::packed::CellOutput::new_builder()
        .type_(Some(type_script).pack())
        .build();

    let celldep_out_point = fake_outpoint();
    let header = fake_header_view(0, random_u64(), random_u64());
    rpc.insert_fake_cell(
        celldep_out_point.clone(),
        CellOutputEx {
            output,
            data: contract_data,
        },
        Some(header),
    );
}

// ---------------------------------------------------------------------------
// Cell seeding
// ---------------------------------------------------------------------------

/// Seed a user cell with always-success lock (blank args).
pub fn seed_user_cell(rpc: &mut FakeRpcClient, capacity: u64) {
    let lock = always_success_script(vec![]);
    let cell =
        CellOutputEx::new_from_scripts(lock, None, vec![], Some(Capacity::shannons(capacity)))
            .expect("build user cell");
    let header = fake_header_view(1, random_u64(), random_u64());
    rpc.insert_fake_cell(fake_outpoint(), cell, Some(header));
}

/// Seed a user cell with the seller's lock args.
pub fn seed_seller_cell(rpc: &mut FakeRpcClient, capacity: u64) {
    let lock = always_success_script(vec![0x01]);
    let cell =
        CellOutputEx::new_from_scripts(lock, None, vec![], Some(Capacity::shannons(capacity)))
            .expect("build seller cell");
    let header = fake_header_view(1, random_u64(), random_u64());
    rpc.insert_fake_cell(fake_outpoint(), cell, Some(header));
}

/// Seed a block header at the given block number.
pub fn seed_header(rpc: &mut FakeRpcClient, block_number: u64, timestamp: u64) {
    rpc.insert_fake_header(fake_header_view(block_number, timestamp, random_u64()));
}

/// Seed a Fiber channel cell referenced by match operations.
pub fn seed_channel_cell(rpc: &mut FakeRpcClient, outpoint: &OutPoint, capacity: u64) {
    let channel_type = Script::new_builder()
        .code_hash(H256([0xCCu8; 32]).pack())
        .hash_type(ScriptHashType::Data1.into())
        .args(Bytes::new().pack())
        .build();
    let lock = Script::new_builder()
        .code_hash(H256(CONTRACT_MOCK).pack())
        .hash_type(ScriptHashType::Type.into())
        .args(Bytes::copy_from_slice(&[0xABu8; 20]).pack())
        .build();
    let cell = CellOutputEx::new_from_scripts(
        lock,
        Some(channel_type),
        vec![],
        Some(Capacity::shannons(capacity)),
    )
    .expect("build channel cell");
    let packed = PackedOutPoint::new_builder()
        .tx_hash(H256(outpoint.tx_hash).pack())
        .index(outpoint.index.pack())
        .build();
    let header = fake_header_view(CHANNEL_CREATED_BLOCK, random_u64(), random_u64());
    rpc.insert_fake_cell(packed, cell, Some(header));
}

/// Seed a Match cell and return the MatchInfo representing it.
pub async fn seed_match(
    rpc: &mut FakeRpcClient,
    match_args: &MatchArgs,
    match_data: &MatchData,
    capacity: u64,
    match_current_block: u64,
) -> MatchInfo {
    let skeleton = celldeps_prepared_skeleton(rpc).await.unwrap();
    let lock = {
        use ckb_cinnabar_calculator::skeleton::ScriptEx;
        ScriptEx::Reference("opticrum".into(), match_args.to_bytes().to_vec())
            .to_script(&skeleton)
            .unwrap()
    };
    let cell = CellOutputEx::new_from_scripts(
        lock,
        None,
        match_data.to_bytes().to_vec(),
        Some(Capacity::shannons(capacity)),
    )
    .unwrap();
    let outpoint = fake_outpoint();
    let header = fake_header_view(match_current_block, random_u64(), random_u64());
    rpc.insert_fake_cell(outpoint.clone(), cell, Some(header));
    MatchInfo {
        match_args: match_args.clone(),
        match_data: match_data.clone(),
        xudt: None,
        ckb_capacity: capacity,
        match_outpoint: OutPoint::new(outpoint.tx_hash().unpack(), outpoint.index().unpack()),
        match_current_block,
    }
}

/// Seed an Order cell and return the OrderInfo representing it.
pub async fn seed_order(
    rpc: &mut FakeRpcClient,
    order_args: &OrderArgs,
    order_data: &OrderData,
    capacity: u64,
) -> OrderInfo {
    let skeleton = celldeps_prepared_skeleton(rpc).await.unwrap();
    let lock = {
        use ckb_cinnabar_calculator::skeleton::ScriptEx;
        ScriptEx::Reference("opticrum".into(), order_args.to_bytes().to_vec())
            .to_script(&skeleton)
            .unwrap()
    };
    let cell = CellOutputEx::new_from_scripts(
        lock,
        None,
        order_data.to_bytes().to_vec(),
        Some(Capacity::shannons(capacity)),
    )
    .unwrap();
    let outpoint = fake_outpoint();
    let header = fake_header_view(ORDER_CREATED_BLOCK, random_u64(), random_u64());
    rpc.insert_fake_cell(outpoint.clone(), cell, Some(header));
    OrderInfo {
        order_args: order_args.clone(),
        order_data: order_data.clone(),
        xudt: None,
        ckb_capacity: capacity,
        order_outpoint: OutPoint::new(outpoint.tx_hash().unpack(), outpoint.index().unpack()),
        fiber_address: None,
    }
}
