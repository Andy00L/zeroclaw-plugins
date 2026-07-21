//! Error type shared by every module in this crate.
//!
//! Errors are values: every fallible function returns `Result<_, CoreError>`
//! and callers branch on the variant. Distinct failure modes carry distinct
//! variants and distinct messages, so a plugin can tell an operator exactly
//! what went wrong (rate limited is not the same as account missing).

use core::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// The host HTTP transport failed before any HTTP status existed
    /// (DNS resolution, connection, request write).
    TransportFailed(String),
    /// The RPC endpoint answered with a non-2xx HTTP status.
    HttpStatus(u16),
    /// The node returned a JSON-RPC error object (for example code 429
    /// "Too many requests" on public endpoints).
    RpcError { code: i64, message: String },
    /// The response parsed as JSON but did not have the shape the Solana
    /// RPC documentation promises.
    MalformedResponse(String),
    /// The queried account does not exist on chain.
    AccountNotFound(String),
    /// The input is not a valid base58-encoded 32-byte public key.
    InvalidPubkey(String),
    /// A blockhash string failed to parse as a base58 32-byte hash.
    InvalidBlockhash(String),
    /// The amount string is not a valid decimal for the given precision.
    InvalidAmount(String),
    /// The account exists but is not a token mint owned by a token program.
    NotAMint(String),
    /// The nonce account is missing, uninitialized, or a legacy version.
    /// Legacy nonce state no longer validates durable transactions
    /// (sourceRef: solana-nonce-3.2.1/src/versions.rs, verify_recent_blockhash).
    InvalidNonceAccount(String),
    /// Compiling instructions into a v0 message failed.
    TxCompileFailed(String),
}

impl fmt::Display for CoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::TransportFailed(detail) => {
                write!(formatter, "HTTP transport failed: {detail}")
            }
            CoreError::HttpStatus(status) => {
                write!(formatter, "RPC endpoint returned HTTP status {status}")
            }
            CoreError::RpcError { code, message } => {
                write!(formatter, "RPC error {code}: {message}")
            }
            CoreError::MalformedResponse(detail) => {
                write!(formatter, "malformed RPC response: {detail}")
            }
            CoreError::AccountNotFound(address) => {
                write!(formatter, "account not found on chain: {address}")
            }
            CoreError::InvalidPubkey(input) => {
                write!(formatter, "not a valid Solana address: {input}")
            }
            CoreError::InvalidBlockhash(input) => {
                write!(formatter, "not a valid blockhash: {input}")
            }
            CoreError::InvalidAmount(detail) => {
                write!(formatter, "invalid amount: {detail}")
            }
            CoreError::NotAMint(detail) => {
                write!(formatter, "not a token mint: {detail}")
            }
            CoreError::InvalidNonceAccount(detail) => {
                write!(formatter, "invalid durable nonce account: {detail}")
            }
            CoreError::TxCompileFailed(detail) => {
                write!(formatter, "transaction compilation failed: {detail}")
            }
        }
    }
}

impl std::error::Error for CoreError {}
