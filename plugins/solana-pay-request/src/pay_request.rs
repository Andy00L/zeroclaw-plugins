//! Pure payment-request core: no wasm dependency, fully host-testable.
//!
//! Safety properties enforced here, in Rust, where no prompt can reach them:
//! the recipient wallet comes ONLY from operator config (there is no
//! recipient argument, so a hijacked model cannot redirect payments), tokens
//! resolve only through the operator-extendable symbol map (no raw mints
//! from the model), amounts are validated integer math, and free-text fields
//! are percent-encoded so they cannot inject URL parameters.

use std::collections::HashMap;

use solana_wasip2_core::addresses::{parse_pubkey, Pubkey};
use solana_wasip2_core::amount::canonicalize_decimal_amount;
use solana_wasip2_core::error::CoreError;
use solana_wasip2_core::pay_url::{build_transfer_request_url, TransferRequest};
use solana_wasip2_core::shape::sanitize_untrusted_text;

/// Mainnet USDC mint and decimals (sourceRef: Circle,
/// https://developers.circle.com/stablecoins/usdc-on-main-networks).
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const USDC_DECIMALS: u8 = 6;
/// Native SOL precision in decimals (sourceRef:
/// https://docs.solanapay.com/spec, amount field).
const SOL_DECIMALS: u8 = 9;

/// Longest free-text field accepted before sanitization, in characters.
const MAX_TEXT_FIELD_CHARS: usize = 64;

/// One entry of the token symbol map: `None` mint means native SOL.
#[derive(Debug, Clone)]
pub struct TokenEntry {
    pub mint: Option<Pubkey>,
    pub decimals: u8,
}

/// Operator configuration from the plugin's jailed config section.
pub struct PayRequestConfig {
    /// The only wallet payments can be requested into. Not configurable by
    /// the model, by design.
    pub recipient: Option<Pubkey>,
    /// Uppercased symbol -> token entry. Always contains USDC and SOL;
    /// operators may add entries via the `tokens` key.
    pub tokens: HashMap<String, TokenEntry>,
}

impl PayRequestConfig {
    /// Build from the flat string map the host injects. An empty section
    /// yields the built-in token map and no recipient, which makes every
    /// request fail closed with a setup instruction.
    pub fn from_section(section: &HashMap<String, String>) -> Result<Self, String> {
        let recipient = match section.get("recipient").filter(|value| !value.is_empty()) {
            Some(recipient_text) => Some(
                parse_pubkey(recipient_text)
                    .map_err(|_| format!("config error: recipient '{recipient_text}' is not a valid address"))?,
            ),
            None => None,
        };

        let mut tokens: HashMap<String, TokenEntry> = HashMap::new();
        tokens.insert(
            "USDC".to_string(),
            TokenEntry {
                mint: Some(parse_pubkey(USDC_MINT).expect("constant mint must parse")),
                decimals: USDC_DECIMALS,
            },
        );
        tokens.insert(
            "SOL".to_string(),
            TokenEntry {
                mint: None,
                decimals: SOL_DECIMALS,
            },
        );

        // Operator extensions: "PYUSD=2b1k...:6,BRZ=FtgG...:4".
        if let Some(token_list) = section.get("tokens").filter(|value| !value.is_empty()) {
            for token_definition in token_list.split(',') {
                let (symbol, entry) = parse_token_definition(token_definition)?;
                tokens.insert(symbol, entry);
            }
        }
        Ok(Self { recipient, tokens })
    }
}

/// Parse one "SYMBOL=MINT:DECIMALS" config entry with distinct errors.
fn parse_token_definition(token_definition: &str) -> Result<(String, TokenEntry), String> {
    let trimmed_definition = token_definition.trim();
    let (symbol_text, mint_and_decimals) = trimmed_definition.split_once('=').ok_or(format!(
        "config error: token entry '{trimmed_definition}' must look like SYMBOL=MINT:DECIMALS"
    ))?;
    let (mint_text, decimals_text) = mint_and_decimals.split_once(':').ok_or(format!(
        "config error: token entry '{trimmed_definition}' is missing ':DECIMALS'"
    ))?;
    let mint = parse_pubkey(mint_text)
        .map_err(|_| format!("config error: token mint '{mint_text}' is not a valid address"))?;
    let decimals: u8 = decimals_text.trim().parse().map_err(|_| {
        format!("config error: token decimals '{decimals_text}' is not a number")
    })?;
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
        TokenEntry {
            mint: Some(mint),
            decimals,
        },
    ))
}

/// Model-facing arguments. Unknown fields are rejected outright, so a
/// prompt-injected "recipient" argument produces a loud error instead of a
/// silent ignore. `__config` is host-injected and never in the schema.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteArgs {
    pub amount: String,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub memo: Option<String>,
    #[serde(rename = "__config", default)]
    pub config: HashMap<String, String>,
}

/// Mirror of the WIT `tool-result` record.
#[derive(Debug)]
pub struct ToolOutcome {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}

impl ToolOutcome {
    fn succeed(output: String) -> Self {
        Self { success: true, output, error: None }
    }
    fn fail(error_message: String) -> Self {
        Self { success: false, output: String::new(), error: Some(error_message) }
    }
}

/// Build the payment request. `generate_reference_bytes` supplies entropy for
/// the per-payment reference key: the shim wires WASI randomness, host tests
/// inject fixed bytes.
pub fn execute_pay_request(
    args_json: &str,
    generate_reference_bytes: impl Fn() -> [u8; 32],
) -> ToolOutcome {
    let args: ExecuteArgs = match serde_json::from_str(args_json) {
        Ok(parsed_args) => parsed_args,
        Err(parse_error) => return ToolOutcome::fail(format!("invalid arguments: {parse_error}")),
    };
    let config = match PayRequestConfig::from_section(&args.config) {
        Ok(config) => config,
        Err(config_error) => return ToolOutcome::fail(config_error),
    };
    let Some(recipient) = config.recipient else {
        return ToolOutcome::fail(
            "no recipient configured: the operator must set `recipient` in this plugin's \
             config section before payment requests can be created"
                .to_string(),
        );
    };

    let requested_symbol = args
        .token
        .as_deref()
        .unwrap_or("USDC")
        .trim()
        .to_uppercase();
    let Some(token_entry) = config.tokens.get(&requested_symbol) else {
        let mut known_symbols: Vec<&str> =
            config.tokens.keys().map(String::as_str).collect();
        known_symbols.sort_unstable();
        return ToolOutcome::fail(format!(
            "token '{requested_symbol}' is not configured; configured tokens: {}",
            known_symbols.join(", ")
        ));
    };

    let canonical_amount = match canonicalize_decimal_amount(&args.amount, token_entry.decimals) {
        Ok(canonical_amount) => canonical_amount,
        Err(amount_error @ CoreError::InvalidAmount(_)) => {
            return ToolOutcome::fail(amount_error.to_string());
        }
        Err(other_error) => return ToolOutcome::fail(other_error.to_string()),
    };

    let reference_key = Pubkey::new_from_array(generate_reference_bytes());
    let sanitize_field =
        |field: &Option<String>| -> Option<String> {
            field
                .as_deref()
                .map(|field_text| sanitize_untrusted_text(field_text, MAX_TEXT_FIELD_CHARS))
                .filter(|sanitized| !sanitized.is_empty())
        };

    let transfer_request = TransferRequest {
        recipient,
        amount: Some(canonical_amount.clone()),
        spl_token: token_entry.mint,
        reference: vec![reference_key],
        label: sanitize_field(&args.label),
        message: sanitize_field(&args.message),
        memo: sanitize_field(&args.memo),
    };
    let pay_url = build_transfer_request_url(&transfer_request);

    let mut output_lines = vec![
        "Payment request created.".to_string(),
        format!("Pay URL: {pay_url}"),
        format!("Amount: {canonical_amount} {requested_symbol}"),
        format!("To: {recipient} (operator-configured recipient)"),
        format!("Reference: {reference_key}"),
    ];
    output_lines.push(
        "Confirm receipt by watching the reference address for the incoming transaction."
            .to_string(),
    );
    ToolOutcome::succeed(output_lines.join("\n"))
}
