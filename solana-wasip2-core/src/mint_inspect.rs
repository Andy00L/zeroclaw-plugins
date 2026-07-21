//! Token mint inspection and risk assessment.
//!
//! Consumes `getAccountInfo` jsonParsed output for a mint (classic SPL Token
//! or Token-2022) and produces first a factual [`MintFacts`], then an
//! opinionated [`RiskAssessment`] with a red/amber/green level and one short
//! reason per finding. The assessment is deliberately conservative: this
//! logic also gates transfers in spl-transfer-build, so unknown or dangerous
//! states must fail toward caution, never toward "fine".

use serde_json::Value;

use crate::addresses::TOKEN_2022_PROGRAM_ID;
use crate::error::CoreError;
use crate::rpc::{LargestTokenAccount, ParsedAccountInfo};

/// Which program owns the mint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenProgramKind {
    SplToken,
    SplToken2022,
}

/// Transfer fee facts from the Token-2022 transferFeeConfig extension.
#[derive(Debug)]
pub struct TransferFeeFacts {
    pub basis_points: u16,
    pub maximum_fee_base_units: u64,
    /// A config authority exists, so the fee can be raised later even if it
    /// is currently zero.
    pub authority_present: bool,
}

/// Factual description of a mint, parsed from jsonParsed account data.
#[derive(Debug)]
pub struct MintFacts {
    pub mint_address: String,
    pub program: TokenProgramKind,
    pub decimals: u8,
    pub supply_base_units: u128,
    pub mint_authority: Option<String>,
    pub freeze_authority: Option<String>,
    /// From the Token-2022 tokenMetadata extension, when present.
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub permanent_delegate: Option<String>,
    pub transfer_fee: Option<TransferFeeFacts>,
    /// `Some` when the transferHook extension is present; the inner value is
    /// the hook program id, which the issuer may not have installed yet.
    pub transfer_hook: Option<Option<String>>,
    pub default_account_state_frozen: bool,
    pub non_transferable: bool,
    pub confidential_transfers: bool,
    pub mint_close_authority: bool,
    /// Extension labels this parser does not specifically understand.
    pub other_extensions: Vec<String>,
}

/// Holder concentration from getTokenLargestAccounts, in basis points so no
/// floating point touches the math.
#[derive(Debug)]
pub struct HolderConcentration {
    pub top1_basis_points: u32,
    pub top5_basis_points: u32,
    pub reported_accounts: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RiskLevel {
    Green,
    Amber,
    Red,
}

#[derive(Debug)]
pub struct RiskAssessment {
    pub level: RiskLevel,
    /// One short sentence per finding, most severe first.
    pub reasons: Vec<String>,
}

/// Extension labels that are benign for holders and only reported, never
/// scored: pointers and metadata (sourceRef: observed jsonParsed labels on
/// mainnet, tests/fixtures/mint_pyusd_json_parsed.json).
const INFORMATIONAL_EXTENSION_LABELS: [&str; 3] =
    ["metadataPointer", "tokenMetadata", "confidentialTransferFeeConfig"];

/// Parse jsonParsed mint account data into facts. Fails with distinct errors
/// when the account is not a mint or not owned by a token program.
pub fn parse_mint_facts(
    mint_address: &str,
    account: &ParsedAccountInfo,
) -> Result<MintFacts, CoreError> {
    let program = if account.owner_program == spl_token_interface::id().to_string() {
        TokenProgramKind::SplToken
    } else if account.owner_program == TOKEN_2022_PROGRAM_ID.to_string() {
        TokenProgramKind::SplToken2022
    } else {
        return Err(CoreError::NotAMint(format!(
            "{mint_address} is owned by {} which is not a token program",
            account.owner_program
        )));
    };

    let account_kind = account
        .parsed_json
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("");
    if account_kind != "mint" {
        return Err(CoreError::NotAMint(format!(
            "{mint_address} is a token program account of type '{account_kind}', not a mint"
        )));
    }
    let info = account.parsed_json.get("info").ok_or_else(|| {
        CoreError::MalformedResponse("jsonParsed mint is missing the info object".to_string())
    })?;

    let decimals = info
        .get("decimals")
        .and_then(Value::as_u64)
        .and_then(|decimals_u64| u8::try_from(decimals_u64).ok())
        .ok_or_else(|| CoreError::MalformedResponse("mint info is missing decimals".to_string()))?;
    let supply_base_units = info
        .get("supply")
        .and_then(Value::as_str)
        .and_then(|supply_text| supply_text.parse::<u128>().ok())
        .ok_or_else(|| CoreError::MalformedResponse("mint info is missing supply".to_string()))?;

    let mut facts = MintFacts {
        mint_address: mint_address.to_string(),
        program,
        decimals,
        supply_base_units,
        mint_authority: read_optional_string(info, "mintAuthority"),
        freeze_authority: read_optional_string(info, "freezeAuthority"),
        name: None,
        symbol: None,
        permanent_delegate: None,
        transfer_fee: None,
        transfer_hook: None,
        default_account_state_frozen: false,
        non_transferable: false,
        confidential_transfers: false,
        mint_close_authority: false,
        other_extensions: Vec::new(),
    };

    let extension_entries = info
        .get("extensions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for extension_entry in &extension_entries {
        apply_extension(&mut facts, extension_entry);
    }
    Ok(facts)
}

fn read_optional_string(info: &Value, key: &str) -> Option<String> {
    info.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Fold one jsonParsed extension entry into the facts. Unknown labels land in
/// `other_extensions` so nothing silently disappears from the report.
fn apply_extension(facts: &mut MintFacts, extension_entry: &Value) {
    let label = extension_entry
        .get("extension")
        .and_then(Value::as_str)
        .unwrap_or("(unlabeled)");
    let state = extension_entry.get("state").cloned().unwrap_or(Value::Null);
    match label {
        "permanentDelegate" => {
            facts.permanent_delegate = state
                .get("delegate")
                .and_then(Value::as_str)
                .map(str::to_string)
                // A permanentDelegate extension with no readable delegate is
                // still a permanent delegate; never drop the finding.
                .or(Some("(unreadable delegate)".to_string()));
        }
        "transferFeeConfig" => {
            let newer_fee = state.get("newerTransferFee").cloned().unwrap_or(Value::Null);
            facts.transfer_fee = Some(TransferFeeFacts {
                basis_points: newer_fee
                    .get("transferFeeBasisPoints")
                    .and_then(Value::as_u64)
                    .and_then(|fee_u64| u16::try_from(fee_u64).ok())
                    .unwrap_or(u16::MAX),
                maximum_fee_base_units: newer_fee
                    .get("maximumFee")
                    .and_then(Value::as_u64)
                    .unwrap_or(u64::MAX),
                authority_present: state
                    .get("transferFeeConfigAuthority")
                    .and_then(Value::as_str)
                    .is_some(),
            });
        }
        "transferHook" => {
            facts.transfer_hook = Some(
                state
                    .get("programId")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            );
        }
        "defaultAccountState" => {
            facts.default_account_state_frozen =
                state.get("accountState").and_then(Value::as_str) == Some("frozen");
        }
        "nonTransferable" => facts.non_transferable = true,
        "confidentialTransferMint" => facts.confidential_transfers = true,
        "mintCloseAuthority" => facts.mint_close_authority = true,
        "tokenMetadata" => {
            facts.name = state.get("name").and_then(Value::as_str).map(str::to_string);
            facts.symbol = state
                .get("symbol")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        other_label => {
            if !INFORMATIONAL_EXTENSION_LABELS.contains(&other_label) {
                facts.other_extensions.push(other_label.to_string());
            }
        }
    }
}

/// Compute holder concentration in basis points. Returns `None` when the
/// supply is zero (concentration is meaningless) or no accounts came back.
pub fn compute_holder_concentration(
    largest_accounts: &[LargestTokenAccount],
    supply_base_units: u128,
) -> Option<HolderConcentration> {
    if supply_base_units == 0 || largest_accounts.is_empty() {
        return None;
    }
    let top1_sum: u128 = largest_accounts
        .iter()
        .take(1)
        .map(|account| account.amount_base_units)
        .sum();
    let top5_sum: u128 = largest_accounts
        .iter()
        .take(5)
        .map(|account| account.amount_base_units)
        .sum();
    let to_basis_points = |part: u128| -> u32 {
        // part <= supply in sane data; clamp anyway so corrupt RPC data
        // cannot overflow the cast.
        ((part.saturating_mul(10_000)) / supply_base_units).min(10_000) as u32
    };
    Some(HolderConcentration {
        top1_basis_points: to_basis_points(top1_sum),
        top5_basis_points: to_basis_points(top5_sum),
        reported_accounts: largest_accounts.len(),
    })
}

/// Threshold above which a single holder is worth a warning, in basis points.
/// 5000 = 50% of supply in one token account.
const TOP1_CONCENTRATION_WARN_BASIS_POINTS: u32 = 5_000;
/// Threshold for the top five holders combined, in basis points (90%).
const TOP5_CONCENTRATION_WARN_BASIS_POINTS: u32 = 9_000;

/// Score the facts. Red findings first, then amber, then notes.
pub fn assess_mint_risk(
    facts: &MintFacts,
    concentration: Option<&HolderConcentration>,
) -> RiskAssessment {
    let mut red_reasons: Vec<String> = Vec::new();
    let mut amber_reasons: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();

    if let Some(delegate) = &facts.permanent_delegate {
        red_reasons.push(format!(
            "permanent delegate {delegate} can transfer or burn any holder's tokens without consent"
        ));
    }
    if let Some(hook_program) = &facts.transfer_hook {
        match hook_program {
            Some(program_id) => amber_reasons.push(format!(
                "a transfer hook program ({program_id}) runs on every transfer; this tool cannot audit it"
            )),
            None => amber_reasons.push(
                "the transfer hook extension is enabled with no program installed; the authority can add one later"
                    .to_string(),
            ),
        }
    }
    if let Some(transfer_fee) = &facts.transfer_fee {
        if transfer_fee.basis_points > 0 {
            amber_reasons.push(format!(
                "transfers are taxed {} basis points (max {} base units per transfer)",
                transfer_fee.basis_points, transfer_fee.maximum_fee_base_units
            ));
        } else if transfer_fee.authority_present {
            amber_reasons.push(
                "the transfer fee is currently 0 but a fee authority can raise it".to_string(),
            );
        }
    }
    if facts.default_account_state_frozen {
        amber_reasons.push(
            "new token accounts start frozen until the issuer thaws them (permissioned token)"
                .to_string(),
        );
    }
    if facts.non_transferable {
        amber_reasons.push("tokens are non-transferable (soulbound)".to_string());
    }
    if let Some(freeze_authority) = &facts.freeze_authority {
        amber_reasons.push(format!(
            "freeze authority {freeze_authority} can freeze any holder's token account"
        ));
    }
    if let Some(mint_authority) = &facts.mint_authority {
        amber_reasons.push(format!(
            "mint authority {mint_authority} can mint unlimited new supply"
        ));
    }
    if let Some(holder_concentration) = concentration {
        if holder_concentration.top1_basis_points >= TOP1_CONCENTRATION_WARN_BASIS_POINTS {
            amber_reasons.push(format!(
                "the largest token account holds {}% of supply (may be an exchange or program vault)",
                holder_concentration.top1_basis_points / 100
            ));
        } else if holder_concentration.top5_basis_points >= TOP5_CONCENTRATION_WARN_BASIS_POINTS {
            amber_reasons.push(format!(
                "the five largest token accounts hold {}% of supply",
                holder_concentration.top5_basis_points / 100
            ));
        }
    }
    if facts.mint_close_authority {
        notes.push("a close authority can close the mint once supply reaches zero".to_string());
    }
    if facts.confidential_transfers {
        notes.push("supports confidential (encrypted-amount) transfers".to_string());
    }
    if !facts.other_extensions.is_empty() {
        notes.push(format!(
            "unrecognized extensions present: {}",
            facts.other_extensions.join(", ")
        ));
    }

    let level = if !red_reasons.is_empty() {
        RiskLevel::Red
    } else if !amber_reasons.is_empty() {
        RiskLevel::Amber
    } else {
        RiskLevel::Green
    };
    let mut reasons = red_reasons;
    reasons.append(&mut amber_reasons);
    reasons.append(&mut notes);
    RiskAssessment { level, reasons }
}
