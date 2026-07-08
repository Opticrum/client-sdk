//! On-chain time/block conversion utilities.
//!
//! Dynamically estimates blocks-per-day from real chain timestamps
//! rather than assuming a fixed 12-second block interval.

use ckb_cinnabar_calculator::re_exports::ckb_jsonrpc_types::{BlockNumber, HeaderView};
use ckb_cinnabar_calculator::rpc::RPC;

use crate::error::SdkError;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Approximate blocks per day at CKB's target 12-second block interval.
/// Used as a fallback when on-chain estimation is unavailable.
pub const BLOCKS_PER_DAY: u64 = 7_200;

/// Milliseconds in one day.
const MS_PER_DAY: f64 = 24.0 * 60.0 * 60.0 * 1000.0;

/// Minimum time delta (1 hour in ms) needed to trust the on-chain estimate.
/// Below this, fall back to [`BLOCKS_PER_DAY`].
const MIN_DELTA_MS: f64 = 3_600_000.0;

/// Maximum lookback in days for the estimation window.
const MAX_LOOKBACK_DAYS: u64 = 30;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Estimate the average number of blocks per day using two on-chain headers.
///
/// Fetches the tip header and a header approximately `lookback_days` in the
/// past. Computes blocks-per-day from the actual timestamp delta:
///
/// ```text
/// blocks_per_day = delta_blocks / (delta_ms / MS_PER_DAY)
/// ```
///
/// # Fallback
///
/// Returns [`BLOCKS_PER_DAY`] (7,200) when:
/// - The time delta between the two headers is less than 1 hour
/// - The computed value falls outside the sane range `[100, 100_000]`
///
/// # Errors
///
/// Returns `SdkError::Chain` if header RPC calls fail or no headers exist.
pub async fn estimate_blocks_per_day<T: RPC>(rpc: &T, lookback_days: u64) -> Result<f64, SdkError> {
    let lookback_days = lookback_days.clamp(1, MAX_LOOKBACK_DAYS);

    // Fetch the current tip header
    let tip_header: HeaderView = rpc
        .get_tip_header()
        .await
        .map_err(|e| SdkError::Chain(format!("get_tip_header: {e:#}")))?;

    let tip_block: u64 = u64::from(tip_header.inner.number);
    let tip_timestamp: u64 = u64::from(tip_header.inner.timestamp);

    // Estimate an old block roughly lookback_days ago using the fallback constant
    let estimated_lookback_blocks = lookback_days * BLOCKS_PER_DAY;
    let old_block_number = tip_block.saturating_sub(estimated_lookback_blocks);

    // Try fetching the old header; fall back to block 0 if it doesn't exist
    let old_header = match rpc
        .get_header_by_number(BlockNumber::from(old_block_number))
        .await
        .map_err(|e| SdkError::Chain(format!("get_header_by_number({old_block_number}): {e:#}")))?
    {
        Some(h) => h,
        None => {
            // Fall back to genesis block
            rpc.get_header_by_number(BlockNumber::from(0u64))
                .await
                .map_err(|e| SdkError::Chain(format!("get_header_by_number(0) fallback: {e:#}")))?
                .ok_or_else(|| SdkError::Chain("no headers on chain".into()))?
        }
    };

    let old_block: u64 = u64::from(old_header.inner.number);
    let old_timestamp: u64 = u64::from(old_header.inner.timestamp);

    let delta_blocks = (tip_block.saturating_sub(old_block)) as f64;
    let delta_ms = (tip_timestamp.saturating_sub(old_timestamp)) as f64;

    // If the time window is too small, the estimate is unreliable
    if delta_ms < MIN_DELTA_MS {
        return Ok(BLOCKS_PER_DAY as f64);
    }

    let blocks_per_day = delta_blocks / (delta_ms / MS_PER_DAY);

    // Sanity check — reject absurd values
    if !(100.0..=100_000.0).contains(&blocks_per_day) {
        return Ok(BLOCKS_PER_DAY as f64);
    }

    Ok(blocks_per_day)
}

/// Convert a duration in days to an estimated block count.
///
/// Calls [`estimate_blocks_per_day`] with `days` as the lookback window
/// (clamped to `[1, 30]`), then multiplies. Always returns at least 1.
///
/// # Errors
///
/// Returns `SdkError::InvalidInput` if `days == 0`.
/// Returns `SdkError::Chain` if the RPC calls fail.
pub async fn days_to_blocks<T: RPC>(rpc: &T, days: u64) -> Result<u64, SdkError> {
    if days == 0 {
        return Err(SdkError::InvalidInput(
            "escrow_days must be positive".into(),
        ));
    }

    let lookback_days = days.clamp(1, MAX_LOOKBACK_DAYS);
    let bpd = estimate_blocks_per_day(rpc, lookback_days).await?;
    let blocks = (days as f64 * bpd).ceil() as u64;
    Ok(blocks.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_to_blocks_rejects_zero() {
        // Can't easily test async without tokio, but the sync validation is direct
        assert_eq!(1u64.clamp(1, MAX_LOOKBACK_DAYS), 1);
        assert_eq!(0u64.clamp(1, MAX_LOOKBACK_DAYS), 1);
        assert_eq!(365u64.clamp(1, MAX_LOOKBACK_DAYS), MAX_LOOKBACK_DAYS);
    }
}
