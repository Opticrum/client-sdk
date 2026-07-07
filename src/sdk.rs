//! Core SDK — wraps `opticrum-calculator` into a simplified, unsigned-transaction API.
//!
//! All `build_*` functions return a balanced, unsigned [`TransactionSkeleton`].
//! The consumer must sign and broadcast. The SDK does **not** manage wallets or keys.

use ckb_cinnabar_calculator::rpc::RPC;

#[cfg(not(target_arch = "wasm32"))]
use ckb_cinnabar_calculator::{
    address::Address,
    instruction::{Instruction, TransactionCalculator},
    operation::basic::{AddSecp256k1SighashCellDep, BalanceTransaction},
    re_exports::ckb_types::packed::Script,
    skeleton::{ChangeReceiver, TransactionSkeleton},
};
use opticrum_calculator::{
    reader,
    types::{CompressedPubkey, MatchInfo, OrderInfo},
};

#[cfg(not(target_arch = "wasm32"))]
use opticrum_calculator::{
    calculator,
    types::{MatchArgs, OrderArgs, OrderData},
};

use crate::error::SdkError;

// ---------------------------------------------------------------------------
// OpticrumSdk — main entry point
// ---------------------------------------------------------------------------

/// Client SDK for the Opticrum protocol.
///
/// Generic over the RPC provider `T`, so it works with any chain backend:
/// - `RpcClient` (production, reqwest-based)
/// - `FakeRpcClient` (testing, in-memory)
/// - Custom `RPC` implementations (WASM, etc.)
pub struct OpticrumSdk<T: RPC> {
    rpc: T,
}

impl<T: RPC> OpticrumSdk<T> {
    /// Create a new SDK instance backed by the given RPC provider.
    pub fn new(rpc: T) -> Self {
        Self { rpc }
    }

    /// Return a reference to the underlying RPC provider.
    pub fn rpc(&self) -> &T {
        &self.rpc
    }

    // -----------------------------------------------------------------------
    // Read operations
    // -----------------------------------------------------------------------

    /// Get the current chain tip block number.
    pub async fn get_tip_block(&self) -> Result<u64, SdkError> {
        let tip: ckb_cinnabar_calculator::re_exports::ckb_jsonrpc_types::BlockNumber = self
            .rpc
            .get_tip_block_number()
            .await
            .map_err(|e| SdkError::Chain(format!("get_tip_block_number: {e:#}")))?;
        Ok(tip.into())
    }

    /// Scan all live Order cells on-chain.
    ///
    /// When `fiber_pubkey` is `Some`, the query is narrowed to cells whose
    /// lock args start with the given pubkey. Pass `None` to return all orders.
    pub async fn scan_orders(
        &self,
        fiber_pubkey: Option<CompressedPubkey>,
    ) -> Result<Vec<OrderInfo>, SdkError> {
        reader::scan_orders(&self.rpc, fiber_pubkey)
            .await
            .map_err(|e| SdkError::Scan(format!("scan_orders: {e:#}")))
    }

    /// Scan all live Match cells on-chain.
    ///
    /// When `fiber_pubkey` is `Some`, the query is narrowed. Pass `None` for
    /// all matches.
    pub async fn scan_matches(
        &self,
        fiber_pubkey: Option<CompressedPubkey>,
    ) -> Result<Vec<MatchInfo>, SdkError> {
        reader::scan_matches(&self.rpc, fiber_pubkey)
            .await
            .map_err(|e| SdkError::Scan(format!("scan_matches: {e:#}")))
    }
}

// -----------------------------------------------------------------------
// Write operations — build unsigned transactions
// (not available on WASM — requires secp256k1 signing support)
// -----------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
impl<T: RPC> OpticrumSdk<T> {
    /// Build a balanced unsigned create-order transaction.
    ///
    /// Returns a [`TransactionSkeleton`] with the Opticrum contract celldep,
    /// secp256k1 sighash celldep, buyer input, the Order output cell, and
    /// a change output. The skeleton is balanced — the consumer only needs
    /// to sign and broadcast.
    #[allow(clippy::too_many_arguments)]
    pub async fn build_create_order(
        &self,
        buyer: Address,
        order_args: &OrderArgs,
        order_data: &OrderData,
        rent_capacity: u64,
        xudt_type_script: Option<Script>,
        fiber_address: Option<String>,
    ) -> Result<TransactionSkeleton, SdkError> {
        let prepare = Instruction::<T>::new(vec![Box::new(AddSecp256k1SighashCellDep {})]);
        let create = calculator::create_order::<T>(
            buyer.clone(),
            order_args,
            order_data,
            rent_capacity,
            xudt_type_script,
            fiber_address,
        );
        let balance = Instruction::<T>::new(vec![Box::new(BalanceTransaction {
            balancer: buyer.payload().into(),
            change_receiver: ChangeReceiver::Address(buyer),
            additional_fee_rate: 0,
        })]);

        let (skeleton, _log) = TransactionCalculator::new(vec![prepare, create, balance])
            .new_skeleton(&self.rpc)
            .await
            .map_err(|e| SdkError::Build(format!("create_order: {e:#}")))?;

        Ok(skeleton)
    }

    /// Build a balanced unsigned cancel-order transaction (Burn pattern).
    pub async fn build_cancel_order(
        &self,
        buyer: Address,
        order_info: OrderInfo,
    ) -> Result<TransactionSkeleton, SdkError> {
        let prepare = Instruction::<T>::new(vec![Box::new(AddSecp256k1SighashCellDep {})]);
        let cancel = calculator::cancel_order::<T>(buyer.clone(), order_info);
        let balance = Instruction::<T>::new(vec![Box::new(BalanceTransaction {
            balancer: buyer.payload().into(),
            change_receiver: ChangeReceiver::Address(buyer),
            additional_fee_rate: 0,
        })]);

        let (skeleton, _log) = TransactionCalculator::new(vec![prepare, cancel, balance])
            .new_skeleton(&self.rpc)
            .await
            .map_err(|e| SdkError::Build(format!("cancel_order: {e:#}")))?;

        Ok(skeleton)
    }

    /// Build a balanced unsigned match-order transaction (Order → Match transition).
    pub async fn build_match_order(
        &self,
        seller: Address,
        order_info: OrderInfo,
        match_args: MatchArgs,
    ) -> Result<TransactionSkeleton, SdkError> {
        let prepare = Instruction::<T>::new(vec![Box::new(AddSecp256k1SighashCellDep {})]);
        let match_tx = calculator::match_order::<T>(seller.clone(), order_info, match_args);
        let balance = Instruction::<T>::new(vec![Box::new(BalanceTransaction {
            balancer: seller.payload().into(),
            change_receiver: ChangeReceiver::Address(seller),
            additional_fee_rate: 0,
        })]);

        let (skeleton, _log) = TransactionCalculator::new(vec![prepare, match_tx, balance])
            .new_skeleton(&self.rpc)
            .await
            .map_err(|e| SdkError::Build(format!("match_order: {e:#}")))?;

        Ok(skeleton)
    }

    /// Build a balanced unsigned extract-rent transaction.
    ///
    /// If the match is already exhausted, the calculator automatically
    /// delegates to `destroy_match` — the returned skeleton will be a
    /// destroy transaction instead.
    pub async fn build_extract_rent(
        &self,
        seller: Address,
        match_info: MatchInfo,
        tip_block: u64,
    ) -> Result<TransactionSkeleton, SdkError> {
        let prepare = Instruction::<T>::new(vec![Box::new(AddSecp256k1SighashCellDep {})]);
        let extract = calculator::extract_rent::<T>(seller.clone(), match_info, tip_block);
        let balance = Instruction::<T>::new(vec![Box::new(BalanceTransaction {
            balancer: seller.payload().into(),
            change_receiver: ChangeReceiver::Address(seller),
            additional_fee_rate: 0,
        })]);

        let (skeleton, _log) = TransactionCalculator::new(vec![prepare, extract, balance])
            .new_skeleton(&self.rpc)
            .await
            .map_err(|e| SdkError::Build(format!("extract_rent: {e:#}")))?;

        Ok(skeleton)
    }

    /// Build a balanced unsigned update-match transaction (buyer injects or withdraws).
    ///
    /// Positive `capacity_delta` = inject, negative = withdraw.
    pub async fn build_update_match(
        &self,
        buyer: Address,
        match_info: MatchInfo,
        new_xudt_amount: u128,
        capacity_delta: i64,
    ) -> Result<TransactionSkeleton, SdkError> {
        let prepare = Instruction::<T>::new(vec![Box::new(AddSecp256k1SighashCellDep {})]);
        let update = calculator::update_match_buyer::<T>(
            buyer.clone(),
            match_info,
            new_xudt_amount,
            capacity_delta,
        );
        let balance = Instruction::<T>::new(vec![Box::new(BalanceTransaction {
            balancer: buyer.payload().into(),
            change_receiver: ChangeReceiver::Address(buyer),
            additional_fee_rate: 0,
        })]);

        let (skeleton, _log) = TransactionCalculator::new(vec![prepare, update, balance])
            .new_skeleton(&self.rpc)
            .await
            .map_err(|e| SdkError::Build(format!("update_match: {e:#}")))?;

        Ok(skeleton)
    }

    /// Build a balanced unsigned destroy-match transaction (Burn pattern).
    ///
    /// Returns `SdkError::NotExhausted` if the match is not yet exhausted —
    /// the on-chain verifier would reject it.
    pub async fn build_destroy_match(
        &self,
        seller: Address,
        match_info: MatchInfo,
        tip_block: u64,
    ) -> Result<TransactionSkeleton, SdkError> {
        // Guard: the contract only allows destruction when exhausted
        if !match_info.is_exhausted(tip_block) {
            return Err(SdkError::NotExhausted(
                match_info.ckb_capacity as f64 / opticrum_calculator::config::CKB_DECIMAL as f64,
            ));
        }

        let prepare = Instruction::<T>::new(vec![Box::new(AddSecp256k1SighashCellDep {})]);
        let destroy = calculator::destroy_match::<T>(seller.clone(), match_info, tip_block);
        let balance = Instruction::<T>::new(vec![Box::new(BalanceTransaction {
            balancer: seller.payload().into(),
            change_receiver: ChangeReceiver::Address(seller),
            additional_fee_rate: 0,
        })]);

        let (skeleton, _log) = TransactionCalculator::new(vec![prepare, destroy, balance])
            .new_skeleton(&self.rpc)
            .await
            .map_err(|e| SdkError::Build(format!("destroy_match: {e:#}")))?;

        Ok(skeleton)
    }
}
