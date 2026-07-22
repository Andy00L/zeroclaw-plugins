//! Pure risk-check core: no wasm dependency, fully host-testable.
//!
//! Safety properties enforced here, in Rust, where no prompt can reach them:
//! the RPC endpoint comes only from operator config (the model cannot point
//! the check at a hostile node to fake a verdict), the risk level is computed
//! from parsed chain state (issuer-supplied text cannot change it), and every
//! issuer-controlled string is sanitized and length-bounded before it enters
//! the report the model reads.

use std::collections::HashMap;

use solana_wasip2_core::addresses::parse_pubkey;
use solana_wasip2_core::amount::format_base_units;
use solana_wasip2_core::error::CoreError;
use solana_wasip2_core::http::JsonHttpTransport;
use solana_wasip2_core::mint_inspect::{
    assess_mint_risk, compute_holder_concentration, parse_mint_facts, HolderConcentration,
    MintFacts, RiskAssessment, RiskLevel, TokenProgramKind,
};
use solana_wasip2_core::rpc::RpcClient;
use solana_wasip2_core::shape::{sanitize_untrusted_text, MAX_UNTRUSTED_TEXT_CHARS};

/// Default RPC endpoint when the operator has not configured one. Public and
/// heavily rate limited; operators should set their own `rpc_url`
/// (sourceRef: https://solana.com/docs/references/clusters).
pub const DEFAULT_RPC_URL: &str = "https://api.mainnet-beta.solana.com";

/// Length bound for RPC error text echoed into a report.
const MAX_ERROR_TEXT_CHARS: usize = 120;

/// The complete accepted config surface; anything else is a typo and the
/// plugin refuses to run with it (fail closed, never fail open).
const ACCEPTED_CONFIG_KEYS: [&str; 1] = ["rpc_url"];

/// Operator configuration, resolved from the plugin's jailed config section.
/// An empty section (the unconfigured and the no-`config_read` case) must
/// produce safe defaults.
pub struct RiskCheckConfig {
    pub rpc_url: String,
}

impl RiskCheckConfig {
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
        Ok(Self { rpc_url })
    }
}

/// The model-facing arguments. `__config` is injected by the host after any
/// model-supplied value for that key is deleted; it never appears in the
/// parameters schema.
#[derive(serde::Deserialize)]
pub struct ExecuteArgs {
    pub mint: String,
    #[serde(rename = "__config", default)]
    pub config: HashMap<String, String>,
}

/// Mirror of the WIT `tool-result` record, so the shim is a one-line
/// translation and every path through this core is host-testable.
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

/// Run the full check against a transport. Never panics; every failure mode
/// comes back as a failed [`ToolOutcome`] with a distinct message.
pub fn execute_risk_check<Transport: JsonHttpTransport>(
    transport: Transport,
    args_json: &str,
) -> ToolOutcome {
    let args: ExecuteArgs = match serde_json::from_str(args_json) {
        Ok(parsed_args) => parsed_args,
        Err(parse_error) => {
            return ToolOutcome::fail(format!("invalid arguments: {parse_error}"));
        }
    };
    let config = match RiskCheckConfig::from_section(&args.config) {
        Ok(config) => config,
        Err(config_error) => return ToolOutcome::fail(config_error),
    };
    let mint_address = match parse_pubkey(&args.mint) {
        Ok(parsed_mint) => parsed_mint,
        Err(address_error) => return ToolOutcome::fail(address_error.to_string()),
    };

    let client = RpcClient::new(transport, config.rpc_url);
    let account = match client.get_parsed_account_info(&mint_address) {
        Ok(Some(account)) => account,
        Ok(None) => {
            return ToolOutcome::fail(
                CoreError::AccountNotFound(mint_address.to_string()).to_string(),
            );
        }
        Err(rpc_error) => return ToolOutcome::fail(rpc_error.to_string()),
    };
    let facts = match parse_mint_facts(&mint_address.to_string(), &account) {
        Ok(facts) => facts,
        Err(mint_error) => return ToolOutcome::fail(mint_error.to_string()),
    };

    // Holder concentration is best-effort: the check adds warnings but the
    // core verdict stands without it, and its absence is reported, never
    // silent.
    let (concentration, concentration_note) = match client.get_token_largest_accounts(&mint_address)
    {
        Ok(largest_accounts) => (
            compute_holder_concentration(&largest_accounts, facts.supply_base_units),
            None,
        ),
        Err(concentration_error) => (
            None,
            Some(sanitize_untrusted_text(
                &concentration_error.to_string(),
                MAX_ERROR_TEXT_CHARS,
            )),
        ),
    };

    let assessment = assess_mint_risk(&facts, concentration.as_ref());
    ToolOutcome::succeed(format_report(
        &facts,
        &assessment,
        concentration.as_ref(),
        concentration_note.as_deref(),
    ))
}

fn describe_level(level: RiskLevel) -> &'static str {
    match level {
        RiskLevel::Red => "RED: do not interact without manual review",
        RiskLevel::Amber => "AMBER: centralized or conditional controls present",
        RiskLevel::Green => "GREEN: no control flags found (not an endorsement)",
    }
}

fn describe_program(program: &TokenProgramKind) -> &'static str {
    match program {
        TokenProgramKind::SplToken => "SPL Token",
        TokenProgramKind::SplToken2022 => "Token-2022",
    }
}

/// Shape the report for an LLM context window: a few hundred tokens, one
/// finding per line, issuer text sanitized and marked as unverified.
fn format_report(
    facts: &MintFacts,
    assessment: &RiskAssessment,
    concentration: Option<&HolderConcentration>,
    concentration_note: Option<&str>,
) -> String {
    let mut report_lines: Vec<String> = Vec::new();
    report_lines.push(format!("Token risk: {}", describe_level(assessment.level)));

    if facts.name.is_some() || facts.symbol.is_some() {
        let sanitized_name = sanitize_untrusted_text(
            facts.name.as_deref().unwrap_or("(unnamed)"),
            MAX_UNTRUSTED_TEXT_CHARS,
        );
        let sanitized_symbol = sanitize_untrusted_text(
            facts.symbol.as_deref().unwrap_or(""),
            MAX_UNTRUSTED_TEXT_CHARS,
        );
        report_lines.push(format!(
            "Token: {sanitized_name} ({sanitized_symbol}) [issuer-supplied name, unverified]"
        ));
    }
    report_lines.push(format!(
        "Mint: {} ({}, {} decimals)",
        facts.mint_address,
        describe_program(&facts.program),
        facts.decimals
    ));
    report_lines.push(format!(
        "Supply: {}",
        format_base_units(facts.supply_base_units, facts.decimals)
    ));

    if assessment.reasons.is_empty() {
        report_lines.push("Findings: none".to_string());
    } else {
        report_lines.push("Findings:".to_string());
        for (finding_index, reason) in assessment.reasons.iter().enumerate() {
            report_lines.push(format!("{}. {reason}", finding_index + 1));
        }
    }

    match (concentration, concentration_note) {
        (Some(holder_concentration), _) => report_lines.push(format!(
            "Holder concentration: top account {}%, top 5 accounts {}% (of {} reported)",
            holder_concentration.top1_basis_points / 100,
            holder_concentration.top5_basis_points / 100,
            holder_concentration.reported_accounts
        )),
        (None, Some(note)) => {
            report_lines.push(format!("Holder concentration unavailable: {note}"));
        }
        (None, None) => {
            report_lines.push("Holder concentration: not computable (zero supply)".to_string());
        }
    }

    report_lines.join("\n")
}
