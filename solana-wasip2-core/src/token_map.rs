//! Operator-configured token symbol maps.
//!
//! Payment plugins never accept raw mint addresses from the model: tokens
//! resolve only through a symbol map the operator controls. USDC and native
//! SOL are built in; operators add entries via a `tokens` config value of
//! comma-separated `SYMBOL=MINT:DECIMALS` definitions.

use std::collections::HashMap;

use crate::addresses::{parse_pubkey, Pubkey};

/// Mainnet USDC mint and decimals (sourceRef: Circle,
/// https://developers.circle.com/stablecoins/usdc-on-main-networks).
pub const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
pub const USDC_DECIMALS: u8 = 6;
/// Native SOL precision in decimals (sourceRef:
/// https://docs.solanapay.com/spec, amount field).
pub const SOL_DECIMALS: u8 = 9;

/// One entry of the token symbol map: `None` mint means native SOL.
#[derive(Debug, Clone)]
pub struct SymbolTokenEntry {
    pub mint: Option<Pubkey>,
    pub decimals: u8,
}

/// The map every payment plugin starts from: USDC and native SOL.
pub fn built_in_symbol_map() -> HashMap<String, SymbolTokenEntry> {
    let mut symbol_map = HashMap::new();
    symbol_map.insert(
        "USDC".to_string(),
        SymbolTokenEntry {
            mint: Some(parse_pubkey(USDC_MINT).expect("constant mint must parse")),
            decimals: USDC_DECIMALS,
        },
    );
    symbol_map.insert(
        "SOL".to_string(),
        SymbolTokenEntry {
            mint: None,
            decimals: SOL_DECIMALS,
        },
    );
    symbol_map
}

/// Parse one "SYMBOL=MINT:DECIMALS" config entry with distinct errors.
pub fn parse_symbol_token_definition(
    token_definition: &str,
) -> Result<(String, SymbolTokenEntry), String> {
    let trimmed_definition = token_definition.trim();
    let (symbol_text, mint_and_decimals) = trimmed_definition.split_once('=').ok_or(format!(
        "config error: token entry '{trimmed_definition}' must look like SYMBOL=MINT:DECIMALS"
    ))?;
    let (mint_text, decimals_text) = mint_and_decimals.split_once(':').ok_or(format!(
        "config error: token entry '{trimmed_definition}' is missing ':DECIMALS'"
    ))?;
    let mint = parse_pubkey(mint_text)
        .map_err(|_| format!("config error: token mint '{mint_text}' is not a valid address"))?;
    let decimals: u8 = decimals_text
        .trim()
        .parse()
        .map_err(|_| format!("config error: token decimals '{decimals_text}' is not a number"))?;
    // Solana Pay caps amount precision at 9 decimals; a larger value here is
    // almost certainly a typo (sourceRef: https://docs.solanapay.com/spec).
    if decimals > 9 {
        return Err(format!(
            "config error: token decimals {decimals} exceeds the Solana Pay maximum of 9"
        ));
    }
    let symbol = symbol_text.trim().to_uppercase();
    if symbol.is_empty() {
        return Err(format!(
            "config error: token entry '{trimmed_definition}' has an empty symbol"
        ));
    }
    Ok((
        symbol,
        SymbolTokenEntry {
            mint: Some(mint),
            decimals,
        },
    ))
}

/// Fold a comma-separated `tokens` config value into an existing map.
pub fn extend_symbol_map_from_config(
    symbol_map: &mut HashMap<String, SymbolTokenEntry>,
    tokens_config_value: &str,
) -> Result<(), String> {
    for token_definition in tokens_config_value.split(',') {
        let (symbol, entry) = parse_symbol_token_definition(token_definition)?;
        symbol_map.insert(symbol, entry);
    }
    Ok(())
}
