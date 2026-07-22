//! Pure payment-watch core: no wasm dependency, fully host-testable.
//!
//! The honest settlement rule this plugin exists for: a transaction that
//! merely references an address is NOT a payment (anyone can attach a
//! merchant's reference key to a worthless transaction). Settlement is
//! verified by the recipient's balance delta inside each transaction's
//! metadata, and the recipient comes only from operator config, so an
//! injected model cannot ask "did the attacker get paid" and hear yes.
//!
//! Statelessness: the host runs each call in a fresh store, so the plugin
//! keeps no memory. The default call rescans the reference's full history
//! (a reference key is per-invoice, so its history IS the invoice). The
//! optional `cursor` argument narrows the scan to signatures newer than a
//! previous call's newest, for alert-style cron polling.

use std::collections::HashMap;

use solana_wasip2_core::addresses::parse_pubkey;
use solana_wasip2_core::amount::{format_base_units, parse_amount_to_base_units};
use solana_wasip2_core::http::JsonHttpTransport;
use solana_wasip2_core::payment_verify::{
    compute_recipient_lamport_delta, compute_recipient_token_delta, transaction_failed,
};
use solana_wasip2_core::rpc::RpcClient;
use solana_wasip2_core::token_map::{
    built_in_symbol_map, extend_symbol_map_from_config, SymbolTokenEntry,
};

/// Default RPC endpoint when the operator has not configured one
/// (sourceRef: https://solana.com/docs/references/clusters).
pub const DEFAULT_RPC_URL: &str = "https://api.mainnet-beta.solana.com";

/// Signatures fetched per call. A per-invoice reference sees few
/// transactions; 20 covers bursts while keeping the response small.
const SIGNATURE_FETCH_LIMIT: u16 = 20;
/// Evidence lines shown in the report; more would waste context tokens.
const MAX_EVIDENCE_LINES: usize = 3;
/// Base58 transaction signatures are 64 bytes, encoding to 86 to 88
/// characters; accept a safety margin below for unusual encodings.
const SIGNATURE_MIN_CHARS: usize = 43;
const SIGNATURE_MAX_CHARS: usize = 88;

/// The complete accepted config surface; anything else is a typo and the
/// plugin refuses to run with it (fail closed, never fail open).
const ACCEPTED_CONFIG_KEYS: [&str; 3] = ["recipient", "rpc_url", "tokens"];

/// Operator configuration from the plugin's jailed config section.
pub struct WatchConfig {
    pub rpc_url: String,
    /// The wallet whose incoming balance proves settlement. Config-only:
    /// the model cannot choose whose payments are verified.
    pub recipient: Option<solana_wasip2_core::addresses::Pubkey>,
    pub tokens: HashMap<String, SymbolTokenEntry>,
}

impl WatchConfig {
    pub fn from_section(section: &HashMap<String, String>) -> Result<Self, String> {
        let unknown_keys =
            solana_wasip2_core::config::find_unknown_config_keys(section, &ACCEPTED_CONFIG_KEYS);
        if !unknown_keys.is_empty() {
            return Err(solana_wasip2_core::config::describe_unknown_config_keys(
                &unknown_keys,
                &ACCEPTED_CONFIG_KEYS,
            ));
        }
        let rpc_url = section
            .get("rpc_url")
            .filter(|configured_url| !configured_url.is_empty())
            .cloned()
            .unwrap_or_else(|| DEFAULT_RPC_URL.to_string());
        let recipient = match section.get("recipient").filter(|value| !value.is_empty()) {
            Some(recipient_text) => Some(parse_pubkey(recipient_text).map_err(|_| {
                format!("config error: recipient '{recipient_text}' is not a valid address")
            })?),
            None => None,
        };
        let mut tokens = built_in_symbol_map();
        if let Some(token_list) = section.get("tokens").filter(|value| !value.is_empty()) {
            extend_symbol_map_from_config(&mut tokens, token_list)?;
        }
        Ok(Self {
            rpc_url,
            recipient,
            tokens,
        })
    }
}

/// Model-facing arguments. Unknown fields are rejected outright. `__config`
/// is host-injected and never in the schema.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteArgs {
    /// The per-invoice reference address from solana-pay-request.
    pub reference: String,
    /// The invoiced amount, decimal user units.
    pub amount: String,
    #[serde(default)]
    pub token: Option<String>,
    /// Newest signature already processed by a previous call, for cron
    /// polling. Omit to rescan the reference's full history.
    #[serde(default)]
    pub cursor: Option<String>,
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
        Self {
            success: true,
            output,
            error: None,
        }
    }
    fn fail(error_message: String) -> Self {
        Self {
            success: false,
            output: String::new(),
            error: Some(error_message),
        }
    }
}

/// A cursor is echoed into an RPC parameter, so it is validated as strictly
/// base58-shaped before any request is built.
fn validate_cursor_signature(cursor_text: &str) -> Result<(), String> {
    let length = cursor_text.chars().count();
    if !(SIGNATURE_MIN_CHARS..=SIGNATURE_MAX_CHARS).contains(&length)
        || !cursor_text.chars().all(|character| {
            character.is_ascii_alphanumeric() && !matches!(character, '0' | 'O' | 'I' | 'l')
        })
    {
        return Err(format!(
            "cursor '{cursor_text}' is not a base58 transaction signature"
        ));
    }
    Ok(())
}

/// Run the watch. Never panics; every failure mode is a distinct message.
pub fn execute_payment_watch<Transport: JsonHttpTransport>(
    transport: Transport,
    args_json: &str,
) -> ToolOutcome {
    let args: ExecuteArgs = match serde_json::from_str(args_json) {
        Ok(parsed_args) => parsed_args,
        Err(parse_error) => return ToolOutcome::fail(format!("invalid arguments: {parse_error}")),
    };
    let config = match WatchConfig::from_section(&args.config) {
        Ok(config) => config,
        Err(config_error) => return ToolOutcome::fail(config_error),
    };
    let Some(recipient_wallet) = config.recipient else {
        return ToolOutcome::fail(
            "no recipient configured: the operator must set `recipient` in this plugin's \
             config section; settlement is only ever verified against the operator's wallet"
                .to_string(),
        );
    };

    let reference_address = match parse_pubkey(&args.reference) {
        Ok(parsed_reference) => parsed_reference,
        Err(address_error) => return ToolOutcome::fail(address_error.to_string()),
    };
    let requested_symbol = args
        .token
        .as_deref()
        .unwrap_or("USDC")
        .trim()
        .to_uppercase();
    let Some(token_entry) = config.tokens.get(&requested_symbol) else {
        let mut known_symbols: Vec<&str> = config.tokens.keys().map(String::as_str).collect();
        known_symbols.sort_unstable();
        return ToolOutcome::fail(format!(
            "token '{requested_symbol}' is not configured; configured tokens: {}",
            known_symbols.join(", ")
        ));
    };
    let expected_base_units = match parse_amount_to_base_units(&args.amount, token_entry.decimals) {
        Ok(expected_base_units) => expected_base_units,
        Err(amount_error) => return ToolOutcome::fail(amount_error.to_string()),
    };
    if let Some(cursor_text) = &args.cursor {
        if let Err(cursor_error) = validate_cursor_signature(cursor_text) {
            return ToolOutcome::fail(cursor_error);
        }
    }

    let client = RpcClient::new(transport, config.rpc_url);
    let signature_records = match client.get_signatures_for_address(
        &reference_address,
        args.cursor.as_deref(),
        SIGNATURE_FETCH_LIMIT,
    ) {
        Ok(signature_records) => signature_records,
        Err(rpc_error) => return ToolOutcome::fail(rpc_error.to_string()),
    };

    let recipient_text = recipient_wallet.to_string();
    let mint_text = token_entry.mint.map(|mint| mint.to_string());
    let mut total_received_base_units: u128 = 0;
    let mut settling_transaction_count: usize = 0;
    let mut evidence_lines: Vec<String> = Vec::new();
    let mut skipped_failed_count: usize = 0;
    let mut unavailable_count: usize = 0;

    for signature_record in &signature_records {
        if signature_record.failed {
            skipped_failed_count += 1;
            continue;
        }
        let transaction_json = match client.get_transaction_json(&signature_record.signature) {
            Ok(Some(transaction_json)) => transaction_json,
            Ok(None) => {
                unavailable_count += 1;
                continue;
            }
            Err(rpc_error) => return ToolOutcome::fail(rpc_error.to_string()),
        };
        if transaction_failed(&transaction_json) {
            skipped_failed_count += 1;
            continue;
        }
        let delta_result = match &mint_text {
            Some(mint) => compute_recipient_token_delta(&transaction_json, &recipient_text, mint),
            None => compute_recipient_lamport_delta(&transaction_json, &recipient_text),
        };
        let recipient_delta = match delta_result {
            Ok(recipient_delta) => recipient_delta,
            Err(parse_error) => return ToolOutcome::fail(parse_error.to_string()),
        };
        if recipient_delta > 0 {
            total_received_base_units += recipient_delta as u128;
            settling_transaction_count += 1;
            if evidence_lines.len() < MAX_EVIDENCE_LINES {
                evidence_lines.push(format!(
                    "  {} (slot {}): +{} {requested_symbol}",
                    signature_record.signature,
                    signature_record.slot,
                    format_base_units(recipient_delta as u128, token_entry.decimals),
                ));
            }
        }
    }

    let status_line = if total_received_base_units >= expected_base_units as u128 {
        "Payment status: PAID"
    } else if total_received_base_units > 0 {
        "Payment status: PARTIAL"
    } else {
        "Payment status: PENDING"
    };

    let mut output_lines = vec![
        status_line.to_string(),
        format!(
            "Expected: {} {requested_symbol} to {recipient_text}",
            format_base_units(expected_base_units as u128, token_entry.decimals)
        ),
        format!(
            "Received: {} {requested_symbol} across {settling_transaction_count} settling \
             transaction(s)",
            format_base_units(total_received_base_units, token_entry.decimals),
        ),
    ];
    output_lines.append(&mut evidence_lines);
    if settling_transaction_count > MAX_EVIDENCE_LINES {
        output_lines.push(format!(
            "Note: evidence shows the first {MAX_EVIDENCE_LINES} of \
             {settling_transaction_count} settling transactions"
        ));
    }
    if total_received_base_units > expected_base_units as u128 {
        output_lines.push(format!(
            "Note: overpaid by {} {requested_symbol}",
            format_base_units(
                total_received_base_units - expected_base_units as u128,
                token_entry.decimals
            )
        ));
    }
    if !signature_records.is_empty() && total_received_base_units == 0 {
        output_lines.push(
            "Note: the reference was touched by transaction(s) that moved no value to the \
             recipient; a reference touch is not a payment"
                .to_string(),
        );
    }
    if skipped_failed_count > 0 {
        output_lines.push(format!(
            "Note: skipped {skipped_failed_count} failed transaction(s)"
        ));
    }
    if unavailable_count > 0 {
        output_lines.push(format!(
            "Note: {unavailable_count} transaction(s) not yet queryable on this RPC node; \
             re-check on the next poll"
        ));
    }
    match signature_records.first() {
        Some(newest_record) => output_lines.push(format!(
            "Cursor: {} (pass as the cursor argument on the next poll to scan only newer \
             transactions)",
            newest_record.signature
        )),
        None => {
            if let Some(cursor_text) = &args.cursor {
                output_lines.push(format!("Cursor: {cursor_text} (unchanged, nothing newer)"));
            }
        }
    }
    ToolOutcome::succeed(output_lines.join("\n"))
}
