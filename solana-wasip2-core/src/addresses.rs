//! Well-known program addresses and address parsing helpers.
//!
//! Program ids come from the program interface crates where a lean crate
//! exists. Token-2022 is the exception: its interface crate drags the
//! confidential-transfer proof stack, so the id is declared locally and
//! verified against the program source.

use std::str::FromStr;

// Re-exported so plugin crates need only this crate to name address types.
pub use solana_pubkey::Pubkey;

use crate::error::CoreError;

/// Token-2022 program id.
/// sourceRef: solana-program/token-2022 interface/src/lib.rs,
/// `declare_id!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb")`.
pub const TOKEN_2022_PROGRAM_ID: Pubkey =
    solana_pubkey::pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

/// Classic SPL Token program id, re-exported from its interface crate
/// (sourceRef: spl-token-interface, `declare_id!`).
pub fn token_program_id() -> Pubkey {
    spl_token_interface::id()
}

/// SPL Memo v3 program id, re-exported from its interface crate
/// (sourceRef: spl-memo-interface, module `v3`).
pub fn memo_program_id() -> Pubkey {
    spl_memo_interface::v3::id()
}

/// Parse a base58 address, trimming surrounding whitespace. Returns a
/// distinct error carrying the offending input so plugins can echo it.
pub fn parse_pubkey(input: &str) -> Result<Pubkey, CoreError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(CoreError::InvalidPubkey("(empty)".to_string()));
    }
    Pubkey::from_str(trimmed).map_err(|_| CoreError::InvalidPubkey(trimmed.to_string()))
}

/// Derive the associated token account for a wallet, mint, and token program.
/// Delegates to the ATA interface crate so the derivation seeds stay in sync
/// with the on-chain program.
pub fn derive_associated_token_address(
    wallet: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Pubkey {
    spl_associated_token_account_interface::address::get_associated_token_address_with_program_id(
        wallet,
        mint,
        token_program,
    )
}
