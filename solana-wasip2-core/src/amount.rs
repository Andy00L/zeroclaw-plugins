//! Decimal token amount handling without floating point.
//!
//! Floating point is never acceptable for money: 0.1 + 0.2 does not equal
//! 0.3 in f64. Amounts are parsed from decimal strings into integer base
//! units with checked arithmetic, and formatted back through string math.

use crate::error::CoreError;

/// Maximum decimals a Solana token mint can declare (u8 in the mint account,
/// and the Solana Pay spec caps native SOL amounts at 9 decimals).
/// sourceRef: https://docs.solanapay.com/spec (amount field).
pub const MAX_TOKEN_DECIMALS: u8 = 9;

/// Parse a decimal user-units string (for example "25", "0.5", ".5") into
/// integer base units for a mint with `decimals` precision.
///
/// Rejected inputs, each with a distinct message: empty strings, signs,
/// exponents, group separators, more than one dot, fraction digits beyond
/// `decimals`, zero amounts, and values overflowing u64.
pub fn parse_amount_to_base_units(amount_text: &str, decimals: u8) -> Result<u64, CoreError> {
    let trimmed = amount_text.trim();
    if trimmed.is_empty() {
        return Err(CoreError::InvalidAmount("amount is empty".to_string()));
    }
    if trimmed.chars().any(|character| !character.is_ascii_digit() && character != '.') {
        return Err(CoreError::InvalidAmount(format!(
            "amount '{trimmed}' may only contain digits and one decimal point"
        )));
    }

    let mut dot_split = trimmed.splitn(3, '.');
    let integer_text = dot_split.next().unwrap_or("");
    let fraction_text = dot_split.next().unwrap_or("");
    if dot_split.next().is_some() {
        return Err(CoreError::InvalidAmount(format!(
            "amount '{trimmed}' has more than one decimal point"
        )));
    }
    if integer_text.is_empty() && fraction_text.is_empty() {
        return Err(CoreError::InvalidAmount("amount has no digits".to_string()));
    }
    if fraction_text.len() > decimals as usize {
        return Err(CoreError::InvalidAmount(format!(
            "amount '{trimmed}' has {} fraction digits but the token has {decimals} decimals",
            fraction_text.len()
        )));
    }

    let integer_part: u128 = if integer_text.is_empty() {
        0
    } else {
        integer_text.parse().map_err(|_| {
            CoreError::InvalidAmount(format!("integer part of '{trimmed}' is out of range"))
        })?
    };
    let mut fraction_part: u128 = if fraction_text.is_empty() {
        0
    } else {
        fraction_text.parse().map_err(|_| {
            CoreError::InvalidAmount(format!("fraction part of '{trimmed}' is out of range"))
        })?
    };
    // Scale the fraction up to full precision: ".5" with 6 decimals is 500000.
    for _ in 0..(decimals as usize - fraction_text.len()) {
        fraction_part *= 10;
    }

    let scale: u128 = 10u128.pow(decimals as u32);
    let base_units = integer_part
        .checked_mul(scale)
        .and_then(|scaled_integer| scaled_integer.checked_add(fraction_part))
        .ok_or_else(|| CoreError::InvalidAmount(format!("amount '{trimmed}' overflows")))?;

    if base_units == 0 {
        return Err(CoreError::InvalidAmount("amount must be greater than zero".to_string()));
    }
    u64::try_from(base_units).map_err(|_| {
        CoreError::InvalidAmount(format!("amount '{trimmed}' exceeds the 64-bit token range"))
    })
}

/// Format integer base units back into a canonical decimal string:
/// no leading zeros, no trailing fraction zeros, "0" for zero.
pub fn format_base_units(amount_base_units: u128, decimals: u8) -> String {
    let scale: u128 = 10u128.pow(decimals as u32);
    let integer_part = amount_base_units / scale;
    let fraction_part = amount_base_units % scale;
    if fraction_part == 0 {
        return integer_part.to_string();
    }
    let padded_fraction = format!("{fraction_part:0width$}", width = decimals as usize);
    let trimmed_fraction = padded_fraction.trim_end_matches('0');
    format!("{integer_part}.{trimmed_fraction}")
}

/// Canonicalize a user-supplied decimal string by round-tripping it through
/// base units. Used for Solana Pay URLs so "0.50" becomes "0.5".
pub fn canonicalize_decimal_amount(amount_text: &str, decimals: u8) -> Result<String, CoreError> {
    let base_units = parse_amount_to_base_units(amount_text, decimals)?;
    Ok(format_base_units(base_units as u128, decimals))
}
