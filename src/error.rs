//! Unified error type for the Opticrum SDK.

use std::fmt;

/// Errors that can occur during SDK operations.
#[derive(Debug)]
pub enum SdkError {
    /// A chain RPC call failed.
    Chain(String),
    /// Scanning on-chain cells failed.
    Scan(String),
    /// Building a transaction failed.
    Build(String),
    /// The caller provided invalid input.
    InvalidInput(String),
    /// The match is already exhausted at the given block.
    AlreadyExhausted(u64),
    /// The match is not yet exhausted (remaining capacity in CKB).
    NotExhausted(f64),
    /// The caller is not authorized for this operation.
    NotAuthorized(String),
}

impl fmt::Display for SdkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Chain(msg) => write!(f, "chain error: {msg}"),
            Self::Scan(msg) => write!(f, "scan error: {msg}"),
            Self::Build(msg) => write!(f, "transaction build error: {msg}"),
            Self::InvalidInput(msg) => write!(f, "invalid input: {msg}"),
            Self::AlreadyExhausted(block) => {
                write!(f, "match is already exhausted at block {block}")
            }
            Self::NotExhausted(remaining) => {
                write!(f, "match is not exhausted (remaining: {remaining} CKB)")
            }
            Self::NotAuthorized(msg) => write!(f, "not authorized: {msg}"),
        }
    }
}

impl std::error::Error for SdkError {}
