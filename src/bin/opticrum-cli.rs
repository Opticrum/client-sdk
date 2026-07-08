//! Opticrum CLI — command-line interface for the Opticrum SDK.
//!
//! Builds, signs (via ckb-cli), and broadcasts Opticrum transactions.
//! Displays on-chain data. No wallet management — signing delegates to ckb-cli.

use std::{path::PathBuf, str::FromStr};

use ckb_cinnabar_calculator::{
    address::{Address, AddressPayload},
    instruction::Instruction,
    operation::basic::AddSecp256k1SighashSignaturesWithCkbCli,
    re_exports::{
        ckb_jsonrpc_types::{OutPoint as JsonOutPoint, Uint32},
        ckb_types::{
            packed::{CellOutput as PackedCellOutput, Script},
            prelude::*,
            H256,
        },
    },
    rpc::{Network, RpcClient, RPC},
    skeleton::TransactionSkeleton,
    TransactionCalculator,
};
use clap::{Parser, Subcommand};
use opticrum_calculator::{
    calculator::{annual_yield_to_rent_per_block, rent_per_block_to_annual_yield},
    config::CKB_DECIMAL,
    types::{CompressedPubkey, OrderArgs, OrderData},
};
use opticrum_sdk::{
    dashboard::compute_dashboard, deadline::find_matches_near_exhaustion, sdk::OpticrumSdk,
};

/// Opticrum CLI — decentralized liquidity marketplace client.
#[derive(Parser)]
#[command(name = "opticrum-cli", version, about)]
struct Cli {
    /// CKB RPC URL
    #[arg(long, default_value = "https://testnet.ckbapp.dev")]
    rpc: String,

    /// CKB Indexer URL (defaults to RPC URL if not set)
    #[arg(long)]
    indexer: Option<String>,

    /// Network type: testnet or mainnet
    #[arg(long, default_value = "testnet")]
    network: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List live Order cells on-chain
    ScanOrders {
        /// Filter by buyer Fiber pubkey (hex-encoded, 66 chars)
        #[arg(long)]
        fiber_pubkey: Option<String>,
    },
    /// List live Match cells on-chain
    ScanMatches {
        /// Filter by buyer Fiber pubkey (hex-encoded)
        #[arg(long)]
        fiber_pubkey: Option<String>,
    },
    /// Compute aggregated dashboard statistics
    Dashboard,
    /// Show matches near exhaustion or already exhausted
    Monitor {
        /// Block threshold for "near exhaustion" (default: 7 days = 50400 blocks)
        #[arg(long, default_value = "50400")]
        blocks_threshold: u64,
    },
    /// Build an unsigned create-order transaction
    CreateOrder {
        /// Buyer's CKB address (bech32m, e.g. ckt1...)
        #[arg(long)]
        buyer: String,

        /// Buyer's Fiber pubkey (hex-encoded, 66 chars)
        #[arg(long)]
        fiber_pubkey: String,

        /// Channel capacity in CKB (e.g. 500)
        #[arg(long)]
        channel_capacity_ckb: f64,

        /// Annual yield percentage (e.g. 5 for 5%)
        #[arg(long)]
        annual_yield: f64,

        /// Number of days of rent to pre-fund (e.g. 30 for ~30 days)
        #[arg(long)]
        escrow_days: u64,

        /// xUDT amount (for token-denominated orders)
        #[arg(long)]
        xudt_amount: Option<u128>,

        /// xUDT token category (use "none" for CKB-only orders)
        #[arg(long, default_value = "none")]
        xudt_category: String,

        /// Fiber node multiaddr for peer discovery
        #[arg(long)]
        fiber_address: Option<String>,

        /// Temp directory for ckb-cli tx JSON file
        #[arg(long, default_value = "/tmp")]
        cache_path: PathBuf,
    },
    /// Build a balanced unsigned cancel-order transaction (Burn pattern)
    CancelOrder {
        /// Order outpoint as tx_hash:index (hex)
        #[arg(long)]
        outpoint: String,
    },
    /// Build a balanced unsigned destroy-match transaction (Burn pattern)
    DestroyMatch {
        /// Match outpoint as tx_hash:index (hex)
        #[arg(long)]
        outpoint: String,
    },
}

fn parse_pubkey(hex: &str) -> Result<CompressedPubkey, String> {
    let bytes = hex::decode(hex).map_err(|e| format!("invalid hex: {e}"))?;
    CompressedPubkey::from_slice(&bytes).map_err(|e| e.to_string())
}

/// Parse an outpoint string in the format `tx_hash:index` (e.g. `abcd1234...:0`).
fn parse_outpoint(s: &str) -> Result<(Vec<u8>, u32), String> {
    let (tx_hex, idx_str) = s
        .rsplit_once(':')
        .ok_or_else(|| format!("invalid outpoint '{s}' — expected format: tx_hash:index"))?;
    let tx_bytes = hex::decode(tx_hex).map_err(|e| format!("invalid tx hash hex: {e}"))?;
    let index: u32 = idx_str
        .parse()
        .map_err(|e| format!("invalid outpoint index '{idx_str}': {e}"))?;
    Ok((tx_bytes, index))
}

/// Print a [`TransactionSkeleton`] in a human-readable format.
fn print_skeleton(title: &str, skeleton: &TransactionSkeleton) {
    println!("=== {title} ===\n");
    println!("Inputs ({}):", skeleton.inputs.len());
    for (i, input) in skeleton.inputs.iter().enumerate() {
        let prev = input.input.previous_output();
        let index: u32 = prev.index().unpack();
        println!(
            "  [{}] tx:{} index:{}",
            i,
            hex::encode(prev.tx_hash().as_slice()),
            index
        );
    }
    println!();
    println!("Outputs ({}):", skeleton.outputs.len());
    for (i, output) in skeleton.outputs.iter().enumerate() {
        let cap: u64 = output.output.capacity().unpack();
        println!(
            "  [{}] capacity: {:.2} CKB  lock: {}",
            i,
            cap as f64 / CKB_DECIMAL as f64,
            hex::encode(output.output.lock().as_slice())
        );
    }
    println!();
    println!("Cell Deps ({}):", skeleton.celldeps.len());
    for (i, dep) in skeleton.celldeps.iter().enumerate() {
        let out_point = dep.celldep.out_point();
        let index: u32 = out_point.index().unpack();
        println!(
            "  [{}] name:{}  tx:{} index:{}  dep_type: {:?}",
            i,
            dep.name,
            hex::encode(out_point.tx_hash().as_slice()),
            index,
            dep.celldep.dep_type()
        );
    }
    println!();
    println!("Witnesses ({}):", skeleton.witnesses.len());
    for (i, witness) in skeleton.witnesses.iter().enumerate() {
        if witness.empty {
            println!("  [{}] (empty placeholder)", i);
        } else if witness.traditional {
            println!(
                "  [{}] lock: {} bytes  input_type: {} bytes  output_type: {} bytes",
                i,
                witness.lock.len(),
                witness.input_type.len(),
                witness.output_type.len()
            );
        } else {
            println!("  [{}] plain: {} bytes", i, witness.lock.len());
        }
    }
    println!();
    println!("Note: This is an unsigned skeleton. Sign and broadcast separately.");
}

// ---------------------------------------------------------------------------
// XudtCategory — known xUDT token types
// ---------------------------------------------------------------------------

/// Known xUDT token categories.
///
/// Each variant maps to a type-script triplet (code_hash, hash_type, args).
/// Add real token variants by filling in their script components.
#[derive(Clone, Debug, Default)]
enum XudtCategory {
    /// No xUDT token — plain CKB denomination.
    #[default]
    None,
    // -- example slots (uncomment and fill for real tokens) --
    // /// rCKB on testnet
    // TestnetRckb,
    // /// rCKB on mainnet
    // MainnetRckb,
}

impl XudtCategory {
    /// Map the category to an optional [`Script`].
    ///
    /// Returns `None` for `XudtCategory::None` (CKB-only orders).
    fn to_type_script(&self) -> Option<Script> {
        match self {
            XudtCategory::None => None,
            // XudtCategory::TestnetRckb => Some(
            //     Script::new_builder()
            //         .code_hash(
            //             Byte32::from_slice(&[0x00u8; 32]) // placeholder
            //                 .expect("valid code_hash length"),
            //         )
            //         .hash_type(ScriptHashType::Type.into())
            //         .args(vec![].pack())
            //         .build(),
            // ),
        }
    }
}

impl FromStr for XudtCategory {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "none" => Ok(XudtCategory::None),
            // "testnet-rckb" => Ok(XudtCategory::TestnetRckb),
            // "mainnet-rckb" => Ok(XudtCategory::MainnetRckb),
            other => Err(format!("unknown xUDT category: '{other}'")),
        }
    }
}

/// Fetch a live cell by outpoint, extract its lock script, and derive an
/// [`Address`] using the given network type.
async fn address_from_outpoint<T: RPC>(
    rpc: &T,
    tx_bytes: &[u8],
    index: u32,
    network: Network,
) -> Result<Address, String> {
    let tx_hash = H256::from_slice(tx_bytes).map_err(|e| format!("invalid tx hash: {e}"))?;
    let json_index: Uint32 = index.into();

    let json_outpoint = JsonOutPoint {
        tx_hash,
        index: json_index,
    };

    let cell_with_status = rpc
        .get_live_cell(&json_outpoint, false)
        .await
        .map_err(|e| format!("failed to fetch live cell: {e}"))?;

    let cell = cell_with_status
        .cell
        .ok_or_else(|| "cell not found at outpoint".to_string())?;

    let packed_output: PackedCellOutput = cell.output.into();
    let lock_script = packed_output.lock();

    let payload = AddressPayload::from(lock_script);
    Ok(Address::new(network, payload))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let network = match cli.network.as_str() {
        "testnet" => Network::Testnet,
        "mainnet" => Network::Mainnet,
        other => {
            return Err(format!("unknown network '{other}'. Supported: testnet, mainnet").into())
        }
    };

    let indexer_url = cli.indexer.as_deref().unwrap_or(&cli.rpc);
    let rpc = RpcClient::new(&cli.rpc, Some(indexer_url));
    let sdk = OpticrumSdk::new(rpc);

    match cli.command {
        Commands::ScanOrders { fiber_pubkey } => {
            let pk = fiber_pubkey.as_deref().map(parse_pubkey).transpose()?;
            let orders = sdk.scan_orders(pk).await?;
            println!("Found {} live Order cells:\n", orders.len());
            for (i, o) in orders.iter().enumerate() {
                println!("--- Order #{} ---", i);
                println!(
                    "  outpoint: {}:{}",
                    hex::encode(o.order_outpoint.tx_hash),
                    o.order_outpoint.index
                );
                println!(
                    "  fiber_pubkey: {}",
                    hex::encode(o.order_args.fiber_pubkey.as_bytes())
                );
                println!(
                    "  channel_capacity: {:.2} CKB",
                    o.order_data.channel_capacity as f64 / CKB_DECIMAL as f64
                );
                println!(
                    "  rent_per_block: {} shannons/block",
                    o.order_data.shannons_per_block
                );
                let annual_yield = rent_per_block_to_annual_yield(
                    o.order_data.shannons_per_block,
                    o.order_data.channel_capacity,
                );
                println!("  annual_yield: {:.2}%", annual_yield * 100.0);
                println!(
                    "  rent_capacity: {:.2} CKB",
                    o.ckb_capacity as f64 / CKB_DECIMAL as f64
                );
                if let Some(ref addr) = o.fiber_address {
                    println!("  fiber_address: {}", addr);
                }
                println!();
            }
        }

        Commands::ScanMatches { fiber_pubkey } => {
            let pk = fiber_pubkey.as_deref().map(parse_pubkey).transpose()?;
            let matches = sdk.scan_matches(pk).await?;
            println!("Found {} live Match cells:\n", matches.len());
            for (i, m) in matches.iter().enumerate() {
                println!("--- Match #{} ---", i);
                println!(
                    "  outpoint: {}:{}",
                    hex::encode(m.match_outpoint.tx_hash),
                    m.match_outpoint.index
                );
                println!(
                    "  channel_outpoint: {}:{}",
                    hex::encode(m.match_args.channel_outpoint.tx_hash),
                    m.match_args.channel_outpoint.index
                );
                println!(
                    "  rent_per_block: {} shannons/block",
                    m.match_data.shannons_per_block
                );
                println!(
                    "  remaining_capacity: {:.2} CKB",
                    m.ckb_capacity as f64 / CKB_DECIMAL as f64
                );
                println!(
                    "  last_extraction_block: {}",
                    m.match_data.last_extraction_block
                );
                let tip = sdk.get_tip_block().await?;
                let extractable = m.extraction_amount(tip);
                println!(
                    "  extractable_now: {:.2} CKB",
                    extractable as f64 / CKB_DECIMAL as f64
                );
                println!("  is_exhausted: {}", m.is_exhausted(tip));
                println!();
            }
        }

        Commands::Dashboard => {
            let data = compute_dashboard(&sdk.rpc().clone(), None).await?;
            println!("=== Opticrum Dashboard ===");
            println!("Tip block: {}", data.tip_block);
            println!("Total orders:  {}", data.total_orders);
            println!("Total matches: {}", data.total_matches);
            println!("  Active:    {}", data.active_matches);
            println!("  Exhausted: {}", data.exhausted_matches);
            println!(
                "Total capacity locked: {:.2} CKB",
                data.total_capacity_locked_ckb()
            );
            println!("Avg shannons/block:    {:.0}", data.avg_shannons_per_block);
            println!(
                "Avg annual yield:      {:.2}%",
                data.avg_annual_yield_bps / 100.0
            );
            println!("Near exhaustion:       {}", data.matches_near_exhaustion);

            println!("\nYield distribution:");
            for bucket in &data.yield_distribution.buckets {
                if bucket.count > 0 {
                    println!(
                        "  {}: {} matches, {:.2} CKB",
                        bucket.label,
                        bucket.count,
                        bucket.total_capacity_ckb()
                    );
                }
            }
        }

        Commands::CreateOrder {
            buyer,
            fiber_pubkey,
            channel_capacity_ckb,
            annual_yield,
            escrow_days,
            xudt_amount,
            xudt_category,
            fiber_address,
            cache_path,
        } => {
            let buyer_addr = Address::from_str(&buyer)
                .map_err(|e| format!("invalid buyer address '{buyer}': {e}"))?;

            // Derive lock_hash from the buyer's lock script
            let lock_script = Script::from(&buyer_addr);
            let lock_hash: [u8; 32] = lock_script.calc_script_hash().unpack();

            let channel_capacity = (channel_capacity_ckb * CKB_DECIMAL as f64).round() as u64;

            // Derive rent params: annual_yield (e.g. 5 → 5%) → shannons_per_block
            let shannons_per_block =
                annual_yield_to_rent_per_block(channel_capacity, annual_yield / 100.0);

            // Convert days to blocks using on-chain timestamps for accuracy
            let escrow_blocks = opticrum_sdk::chain::days_to_blocks(sdk.rpc(), escrow_days)
                .await
                .map_err(|e| format!("failed to estimate escrow blocks: {e}"))?;

            let rent_capacity = shannons_per_block * escrow_blocks;

            let order_args = OrderArgs::new(parse_pubkey(&fiber_pubkey)?, lock_hash);
            let order_data = OrderData::new(
                xudt_amount.unwrap_or(0),
                channel_capacity,
                shannons_per_block,
            );

            let signer = buyer_addr.clone();

            let mut skeleton = sdk
                .build_create_order(
                    buyer_addr,
                    &order_args,
                    &order_data,
                    rent_capacity,
                    XudtCategory::from_str(&xudt_category)?.to_type_script(),
                    fiber_address,
                )
                .await?;

            eprintln!("Signing transaction with ckb-cli...");
            let sign_op = AddSecp256k1SighashSignaturesWithCkbCli {
                signer_address: signer,
                cache_path,
                keep_cache_file: false,
            };
            let sign_instruction = Instruction::new(vec![Box::new(sign_op)]);
            TransactionCalculator::new(vec![sign_instruction])
                .apply_skeleton(sdk.rpc(), &mut skeleton)
                .await
                .map_err(|e| format!("ckb-cli signing failed: {e:#}"))?;
            eprintln!("Transaction signed successfully.");

            eprintln!("Broadcasting transaction...");
            let tx_hash = skeleton
                .send_and_wait(sdk.rpc(), 0, None)
                .await
                .map_err(|e| format!("broadcast failed: {e:#}"))?;
            println!("Transaction broadcast successfully!");
            println!("  Tx hash: {tx_hash:#x}");
        }

        Commands::CancelOrder { outpoint } => {
            let (tx_bytes, index) = parse_outpoint(&outpoint)?;

            // Derive signer address from the cell's lock script + network
            let signer =
                address_from_outpoint(sdk.rpc(), &tx_bytes, index, network.clone()).await?;

            let orders = sdk.scan_orders(None).await?;
            let order_info = orders
                .into_iter()
                .find(|o| {
                    o.order_outpoint.tx_hash == tx_bytes.as_slice()
                        && o.order_outpoint.index == index
                })
                .ok_or_else(|| format!("no live Order found at outpoint {outpoint}"))?;

            let skeleton = sdk.build_cancel_order(signer, order_info).await?;
            print_skeleton("Unsigned CancelOrder Transaction", &skeleton);
        }

        Commands::DestroyMatch { outpoint } => {
            let (tx_bytes, index) = parse_outpoint(&outpoint)?;

            // Derive signer address from the cell's lock script + network
            let signer =
                address_from_outpoint(sdk.rpc(), &tx_bytes, index, network.clone()).await?;

            let matches = sdk.scan_matches(None).await?;
            let match_info = matches
                .into_iter()
                .find(|m| {
                    m.match_outpoint.tx_hash == tx_bytes.as_slice()
                        && m.match_outpoint.index == index
                })
                .ok_or_else(|| format!("no live Match found at outpoint {outpoint}"))?;

            let tip = sdk.get_tip_block().await?;

            let skeleton = sdk.build_destroy_match(signer, match_info, tip).await?;
            print_skeleton("Unsigned DestroyMatch Transaction", &skeleton);
        }

        Commands::Monitor { blocks_threshold } => {
            let tip = sdk.get_tip_block().await?;
            let near =
                find_matches_near_exhaustion(&sdk.rpc().clone(), tip, blocks_threshold, None)
                    .await?;

            println!(
                "Matches within {} blocks of exhaustion:\n",
                blocks_threshold
            );
            for d in &near {
                let status = match d.health {
                    opticrum_sdk::types::MatchHealth::Exhausted => "EXHAUSTED",
                    opticrum_sdk::types::MatchHealth::Critical => "CRITICAL",
                    opticrum_sdk::types::MatchHealth::Warning => "WARNING",
                    opticrum_sdk::types::MatchHealth::Healthy => "HEALTHY",
                };
                println!(
                    "  {}  [{status}] {:.1}h remaining  extractable: {:.2} CKB",
                    d.match_outpoint, d.estimated_hours_remaining, d.extractable_now_ckb,
                );
            }
        }
    }

    Ok(())
}
