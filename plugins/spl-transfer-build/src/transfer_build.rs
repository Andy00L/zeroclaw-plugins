//! Pure transfer-builder core: no wasm dependency, fully host-testable.
//!
//! This plugin builds transactions it can never sign or send: the output is
//! an unsigned, base64-encoded v0 transaction for the operator (or the
//! host's approval flow) to inspect and sign. The guardrails live here, in
//! Rust, where no prompt can reach them:
//!
//! - the sender wallet comes only from operator config;
//! - recipients must be on the operator's allowlist, or nothing is built;
//! - tokens resolve only through the operator's symbol map, each entry with
//!   a mandatory per-call cap enforced in base units;
//! - the mint is re-inspected on chain before every build, and dangerous
//!   states (permanent delegate, active transfer hook, frozen-by-default)
//!   refuse to build unless the operator explicitly overrides;
//! - configured decimals must match on-chain decimals, catching both config
//!   typos and mint substitution.

use std::collections::HashMap;

use solana_wasip2_core::addresses::{
    parse_pubkey, token_program_id, Pubkey, TOKEN_2022_PROGRAM_ID,
};
use solana_wasip2_core::amount::{format_base_units, parse_amount_to_base_units};
use solana_wasip2_core::error::CoreError;
use solana_wasip2_core::http::JsonHttpTransport;
use solana_wasip2_core::mint_inspect::{parse_mint_facts, MintFacts, TokenProgramKind};
use solana_wasip2_core::nonce::parse_nonce_account_data;
use solana_wasip2_core::rpc::RpcClient;
use solana_wasip2_core::shape::sanitize_untrusted_text;
use solana_wasip2_core::txbuild::{
    build_spl_transfer_transaction, TransactionLifetime, TransferCheckedSpec,
};

/// Default RPC endpoint when the operator has not configured one
/// (sourceRef: https://solana.com/docs/references/clusters).
pub const DEFAULT_RPC_URL: &str = "https://api.mainnet-beta.solana.com";

/// Mainnet USDC mint and decimals (sourceRef: Circle,
/// https://developers.circle.com/stablecoins/usdc-on-main-networks).
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const USDC_DECIMALS: u8 = 6;
/// Built-in per-call cap for the built-in USDC entry, in user units. Safety
/// is opt-out: operators raise it by redefining USDC under `tokens`.
const DEFAULT_USDC_CAP_UI: &str = "100";

/// Longest memo accepted, in characters, before sanitization.
const MAX_MEMO_CHARS: usize = 120;

/// Length bound for unrecognized extension labels echoed into a refusal.
/// Labels come from the RPC's jsonParsed output, which the operator's node
/// controls; bounding them keeps a hostile node from flooding the message.
const MAX_EXTENSION_LABEL_CHARS: usize = 80;

/// One transferable token: mint, precision, and a hard per-call cap.
#[derive(Debug, Clone)]
pub struct TransferTokenEntry {
    pub mint: Pubkey,
    pub decimals: u8,
    pub max_amount_base_units: u64,
}

/// The complete accepted config surface; anything else is a typo and the
/// plugin refuses to run with it (fail closed, never fail open). A silently
/// ignored `allowed_recipient` (singular) would disable the allowlist.
const ACCEPTED_CONFIG_KEYS: [&str; 6] = [
    "allowed_recipients",
    "nonce_account",
    "override_risk_gate",
    "rpc_url",
    "sender_wallet",
    "tokens",
];

pub struct TransferBuildConfig {
    pub rpc_url: String,
    pub sender_wallet: Option<Pubkey>,
    pub allowed_recipients: Vec<Pubkey>,
    pub tokens: HashMap<String, TransferTokenEntry>,
    pub nonce_account: Option<Pubkey>,
    pub override_risk_gate: bool,
}

impl TransferBuildConfig {
    /// Build from the flat string map the host injects. An empty section
    /// yields a config that can build nothing: no sender, no allowlist.
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

        let sender_wallet = match section
            .get("sender_wallet")
            .filter(|value| !value.is_empty())
        {
            Some(sender_text) => Some(parse_pubkey(sender_text).map_err(|_| {
                format!("config error: sender_wallet '{sender_text}' is not a valid address")
            })?),
            None => None,
        };

        let mut allowed_recipients = Vec::new();
        if let Some(recipient_list) = section.get("allowed_recipients") {
            for recipient_text in recipient_list.split(',') {
                let trimmed_recipient = recipient_text.trim();
                if trimmed_recipient.is_empty() {
                    continue;
                }
                allowed_recipients.push(parse_pubkey(trimmed_recipient).map_err(|_| {
                    format!(
                        "config error: allowed recipient '{trimmed_recipient}' is not a valid address"
                    )
                })?);
            }
        }

        let mut tokens: HashMap<String, TransferTokenEntry> = HashMap::new();
        tokens.insert(
            "USDC".to_string(),
            TransferTokenEntry {
                mint: parse_pubkey(USDC_MINT).expect("constant mint must parse"),
                decimals: USDC_DECIMALS,
                max_amount_base_units: parse_amount_to_base_units(
                    DEFAULT_USDC_CAP_UI,
                    USDC_DECIMALS,
                )
                .expect("constant cap must parse"),
            },
        );
        if let Some(token_list) = section.get("tokens").filter(|value| !value.is_empty()) {
            for token_definition in token_list.split(',') {
                let (symbol, entry) = parse_token_definition(token_definition)?;
                tokens.insert(symbol, entry);
            }
        }

        let nonce_account = match section
            .get("nonce_account")
            .filter(|value| !value.is_empty())
        {
            Some(nonce_text) => Some(parse_pubkey(nonce_text).map_err(|_| {
                format!("config error: nonce_account '{nonce_text}' is not a valid address")
            })?),
            None => None,
        };

        Ok(Self {
            rpc_url,
            sender_wallet,
            allowed_recipients,
            tokens,
            nonce_account,
            override_risk_gate: section
                .get("override_risk_gate")
                .map(|value| value.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
        })
    }
}

/// Parse one "SYMBOL=MINT:DECIMALS:MAX" config entry. The cap is mandatory:
/// there is no way to configure an uncapped token.
fn parse_token_definition(token_definition: &str) -> Result<(String, TransferTokenEntry), String> {
    let trimmed_definition = token_definition.trim();
    let malformed = || {
        format!(
            "config error: token entry '{trimmed_definition}' must look like \
             SYMBOL=MINT:DECIMALS:MAX (the per-call cap is mandatory)"
        )
    };
    let (symbol_text, remainder) = trimmed_definition.split_once('=').ok_or_else(malformed)?;
    let mut remainder_parts = remainder.split(':');
    let mint_text = remainder_parts.next().ok_or_else(malformed)?;
    let decimals_text = remainder_parts.next().ok_or_else(malformed)?;
    let cap_text = remainder_parts.next().ok_or_else(malformed)?;
    if remainder_parts.next().is_some() {
        return Err(malformed());
    }

    let mint = parse_pubkey(mint_text)
        .map_err(|_| format!("config error: token mint '{mint_text}' is not a valid address"))?;
    let decimals: u8 = decimals_text
        .trim()
        .parse()
        .map_err(|_| format!("config error: token decimals '{decimals_text}' is not a number"))?;
    if decimals > 9 {
        return Err(format!(
            "config error: token decimals {decimals} exceeds the supported maximum of 9"
        ));
    }
    let max_amount_base_units = parse_amount_to_base_units(cap_text.trim(), decimals)
        .map_err(|cap_error| format!("config error: token cap '{cap_text}': {cap_error}"))?;
    let symbol = symbol_text.trim().to_uppercase();
    if symbol.is_empty() {
        return Err(format!(
            "config error: token entry '{trimmed_definition}' has an empty symbol"
        ));
    }
    Ok((
        symbol,
        TransferTokenEntry {
            mint,
            decimals,
            max_amount_base_units,
        },
    ))
}

/// Model-facing arguments. Unknown fields (for example a smuggled "sender")
/// are rejected outright. `__config` is host-injected, never in the schema.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteArgs {
    pub recipient: String,
    pub amount: String,
    #[serde(default)]
    pub token: Option<String>,
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

/// The mint states that refuse to build. An operator who accepts one of
/// these knowingly sets `override_risk_gate = "true"`; the reasons still
/// appear in the output.
fn risk_gate_findings(facts: &MintFacts) -> Vec<String> {
    let mut findings = Vec::new();
    if let Some(delegate) = &facts.permanent_delegate {
        findings.push(format!(
            "the mint has a permanent delegate ({delegate}) that can seize tokens"
        ));
    }
    if let Some(Some(hook_program)) = &facts.transfer_hook {
        findings.push(format!(
            "an unaudited transfer hook program ({hook_program}) runs on every transfer"
        ));
    }
    if facts.default_account_state_frozen {
        findings.push(
            "new token accounts start frozen; the recipient may be unable to use the funds"
                .to_string(),
        );
    }
    if facts.non_transferable {
        findings.push("the token is non-transferable".to_string());
    }
    // Fail closed on extensions this tool cannot judge: a future or unknown
    // extension may change transfer semantics (fees, hooks, seizure) in ways
    // the checks above never see.
    if !facts.other_extensions.is_empty() {
        let extension_labels = sanitize_untrusted_text(
            &facts.other_extensions.join(", "),
            MAX_EXTENSION_LABEL_CHARS,
        );
        findings.push(format!(
            "the mint carries extension(s) this tool does not recognize ({extension_labels}); \
             their transfer semantics cannot be verified"
        ));
    }
    findings
}

/// Build the unsigned transfer. Every failure is a distinct, actionable
/// message; nothing is ever signed or sent from here.
pub fn execute_transfer_build<Transport: JsonHttpTransport>(
    transport: Transport,
    args_json: &str,
) -> ToolOutcome {
    let args: ExecuteArgs = match serde_json::from_str(args_json) {
        Ok(parsed_args) => parsed_args,
        Err(parse_error) => return ToolOutcome::fail(format!("invalid arguments: {parse_error}")),
    };
    let config = match TransferBuildConfig::from_section(&args.config) {
        Ok(config) => config,
        Err(config_error) => return ToolOutcome::fail(config_error),
    };

    // Guardrail 1: a sender and a non-empty allowlist must be configured.
    let Some(sender_wallet) = config.sender_wallet else {
        return ToolOutcome::fail(
            "no sender configured: the operator must set `sender_wallet` in this plugin's \
             config section"
                .to_string(),
        );
    };
    if config.allowed_recipients.is_empty() {
        return ToolOutcome::fail(
            "no recipient allowlist configured: the operator must set `allowed_recipients` \
             in this plugin's config section; transfers to arbitrary addresses are never built"
                .to_string(),
        );
    }

    // Guardrail 2: the recipient must be on the allowlist.
    let recipient_wallet = match parse_pubkey(&args.recipient) {
        Ok(parsed_recipient) => parsed_recipient,
        Err(address_error) => return ToolOutcome::fail(address_error.to_string()),
    };
    if !config.allowed_recipients.contains(&recipient_wallet) {
        return ToolOutcome::fail(format!(
            "recipient {recipient_wallet} is not on the operator's allowlist; no transaction built"
        ));
    }

    // Guardrail 3: the token must be configured, with its mandatory cap.
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
    let amount_base_units = match parse_amount_to_base_units(&args.amount, token_entry.decimals) {
        Ok(amount_base_units) => amount_base_units,
        Err(amount_error) => return ToolOutcome::fail(amount_error.to_string()),
    };
    if amount_base_units > token_entry.max_amount_base_units {
        return ToolOutcome::fail(format!(
            "amount {} {requested_symbol} exceeds the per-call cap of {} {requested_symbol}; \
             no transaction built",
            format_base_units(amount_base_units as u128, token_entry.decimals),
            format_base_units(
                token_entry.max_amount_base_units as u128,
                token_entry.decimals
            ),
        ));
    }

    // Guardrail 4: re-inspect the mint on chain before every build.
    let client = RpcClient::new(transport, config.rpc_url);
    let mint_account = match client.get_parsed_account_info(&token_entry.mint) {
        Ok(Some(mint_account)) => mint_account,
        Ok(None) => {
            return ToolOutcome::fail(
                CoreError::AccountNotFound(token_entry.mint.to_string()).to_string(),
            );
        }
        Err(rpc_error) => return ToolOutcome::fail(rpc_error.to_string()),
    };
    let facts = match parse_mint_facts(&token_entry.mint.to_string(), &mint_account) {
        Ok(facts) => facts,
        Err(mint_error) => return ToolOutcome::fail(mint_error.to_string()),
    };
    if facts.decimals != token_entry.decimals {
        return ToolOutcome::fail(format!(
            "configured decimals ({}) do not match the on-chain mint decimals ({}); \
             fix the token config before transferring",
            token_entry.decimals, facts.decimals
        ));
    }
    let gate_findings = risk_gate_findings(&facts);
    if !gate_findings.is_empty() && !config.override_risk_gate {
        return ToolOutcome::fail(format!(
            "risk gate refused to build: {}. Set override_risk_gate = \"true\" in the \
             plugin config to accept this risk explicitly",
            gate_findings.join("; ")
        ));
    }
    let token_program = match facts.program {
        TokenProgramKind::SplToken => token_program_id(),
        TokenProgramKind::SplToken2022 => TOKEN_2022_PROGRAM_ID,
    };

    // The sender must already hold the token; catching this here beats a
    // cryptic on-chain failure after signing.
    let sender_token_account = solana_wasip2_core::addresses::derive_associated_token_address(
        &sender_wallet,
        &token_entry.mint,
        &token_program,
    );
    match client.account_exists(&sender_token_account) {
        Ok(true) => {}
        Ok(false) => {
            return ToolOutcome::fail(format!(
                "the sender wallet has no {requested_symbol} token account \
                 ({sender_token_account}); nothing to transfer from"
            ));
        }
        Err(rpc_error) => return ToolOutcome::fail(rpc_error.to_string()),
    }

    let recipient_token_account = solana_wasip2_core::addresses::derive_associated_token_address(
        &recipient_wallet,
        &token_entry.mint,
        &token_program,
    );
    let create_recipient_ata = match client.account_exists(&recipient_token_account) {
        Ok(recipient_ata_exists) => !recipient_ata_exists,
        Err(rpc_error) => return ToolOutcome::fail(rpc_error.to_string()),
    };

    // Transaction lifetime: a configured durable nonce survives a slow
    // approval queue; otherwise the blockhash gives ~90 seconds to sign.
    let lifetime = match config.nonce_account {
        Some(nonce_account) => {
            let nonce_data = match client.get_account_data(&nonce_account) {
                Ok(Some(nonce_data)) => nonce_data,
                Ok(None) => {
                    return ToolOutcome::fail(
                        CoreError::InvalidNonceAccount(format!(
                            "nonce account {nonce_account} does not exist"
                        ))
                        .to_string(),
                    );
                }
                Err(rpc_error) => return ToolOutcome::fail(rpc_error.to_string()),
            };
            let nonce_info = match parse_nonce_account_data(&nonce_data) {
                Ok(nonce_info) => nonce_info,
                Err(nonce_error) => return ToolOutcome::fail(nonce_error.to_string()),
            };
            if nonce_info.authority != sender_wallet {
                return ToolOutcome::fail(format!(
                    "the nonce account authority ({}) is not the configured sender wallet; \
                     the sender could not sign the nonce advance",
                    nonce_info.authority
                ));
            }
            TransactionLifetime::DurableNonce {
                nonce_account,
                nonce_authority: sender_wallet,
                nonce_value: nonce_info.nonce_value,
            }
        }
        None => match client.get_latest_blockhash() {
            Ok(recent_blockhash) => TransactionLifetime::RecentBlockhash(recent_blockhash),
            Err(rpc_error) => return ToolOutcome::fail(rpc_error.to_string()),
        },
    };
    let lifetime_note = match &lifetime {
        TransactionLifetime::DurableNonce { nonce_account, .. } => format!(
            "durable nonce {nonce_account}: the transaction stays signable until the nonce advances"
        ),
        TransactionLifetime::RecentBlockhash(_) => {
            "recent blockhash: sign within about 90 seconds or the transaction expires \
             (configure nonce_account for a durable lifetime)"
                .to_string()
        }
    };

    let sanitized_memo = args
        .memo
        .as_deref()
        .map(|memo_text| sanitize_untrusted_text(memo_text, MAX_MEMO_CHARS))
        .filter(|sanitized| !sanitized.is_empty());

    let spec = TransferCheckedSpec {
        token_program_id: token_program,
        mint: token_entry.mint,
        sender_wallet,
        recipient_wallet,
        amount_base_units,
        decimals: token_entry.decimals,
        create_recipient_ata,
        memo_text: sanitized_memo.clone(),
    };
    let built = match build_spl_transfer_transaction(&spec, &lifetime) {
        Ok(built) => built,
        Err(build_error) => return ToolOutcome::fail(build_error.to_string()),
    };

    let mut output_lines = vec![
        "Unsigned transfer built. Nothing has been signed or sent.".to_string(),
        format!(
            "Send: {} {requested_symbol} ({})",
            format_base_units(amount_base_units as u128, token_entry.decimals),
            token_entry.mint
        ),
        format!("From: {sender_wallet} (operator-configured sender)"),
        format!("To: {recipient_wallet} (allowlisted)"),
    ];
    if create_recipient_ata {
        output_lines.push(format!(
            "Note: the recipient has no {requested_symbol} account yet; the transaction \
             creates {recipient_token_account} at the sender's expense (rent-exempt minimum)"
        ));
    }
    if let Some(memo_text) = &sanitized_memo {
        output_lines.push(format!("Memo: {memo_text}"));
    }
    output_lines.push(format!("Lifetime: {lifetime_note}"));
    output_lines.push(format!(
        "Signers required: {}",
        built
            .transaction
            .required_signers
            .iter()
            .map(Pubkey::to_string)
            .collect::<Vec<String>>()
            .join(", ")
    ));
    output_lines.push(format!(
        "Unsigned transaction (base64, verify before signing):\n{}",
        built.transaction.unsigned_transaction_base64
    ));
    ToolOutcome::succeed(output_lines.join("\n"))
}
