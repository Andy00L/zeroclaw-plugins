//! Host tests for mint parsing and risk assessment, driven end to end
//! through the RPC client with mainnet-captured fixtures: USDC (classic SPL
//! Token) and PYUSD (Token-2022 with a permanent delegate, transfer fee
//! config, transfer hook extension, and confidential transfers).

mod common;

use common::MockTransport;
use serde_json::json;
use solana_wasip2_core::addresses::parse_pubkey;
use solana_wasip2_core::error::CoreError;
use solana_wasip2_core::mint_inspect::{
    assess_mint_risk, compute_holder_concentration, parse_mint_facts, RiskLevel, TokenProgramKind,
};
use solana_wasip2_core::rpc::{LargestTokenAccount, ParsedAccountInfo, RpcClient};

const MINT_USDC_FIXTURE: &str = include_str!("fixtures/mint_usdc_json_parsed.json");
const MINT_PYUSD_FIXTURE: &str = include_str!("fixtures/mint_pyusd_json_parsed.json");

const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const PYUSD_MINT: &str = "2b1kV6DkPAnxd5ixfnxCpjxmKwqjjaYmCZfHsFu24GXo";
const RPC_URL: &str = "https://rpc.test.invalid";

fn fetch_parsed_account(fixture: &str, mint_address: &str) -> ParsedAccountInfo {
    let transport = MockTransport::from_fixtures(&[fixture]);
    let client = RpcClient::new(transport, RPC_URL);
    client
        .get_parsed_account_info(&parse_pubkey(mint_address).unwrap())
        .unwrap()
        .expect("fixture must contain an account")
}

#[test]
fn usdc_parses_as_a_classic_mint_with_both_authorities() {
    let account = fetch_parsed_account(MINT_USDC_FIXTURE, USDC_MINT);
    let facts = parse_mint_facts(USDC_MINT, &account).unwrap();
    assert_eq!(facts.program, TokenProgramKind::SplToken);
    assert_eq!(facts.decimals, 6);
    assert!(facts.mint_authority.is_some());
    assert!(facts.freeze_authority.is_some());
    assert!(facts.permanent_delegate.is_none());
    assert!(facts.transfer_fee.is_none());
    assert!(facts.other_extensions.is_empty());
}

#[test]
fn usdc_scores_amber_for_issuer_control_only() {
    let account = fetch_parsed_account(MINT_USDC_FIXTURE, USDC_MINT);
    let facts = parse_mint_facts(USDC_MINT, &account).unwrap();
    let assessment = assess_mint_risk(&facts, None);
    assert_eq!(assessment.level, RiskLevel::Amber);
    assert_eq!(assessment.reasons.len(), 2);
    assert!(assessment.reasons[0].contains("freeze authority"));
    assert!(assessment.reasons[1].contains("mint authority"));
}

#[test]
fn pyusd_parses_its_token_2022_extensions() {
    let account = fetch_parsed_account(MINT_PYUSD_FIXTURE, PYUSD_MINT);
    let facts = parse_mint_facts(PYUSD_MINT, &account).unwrap();
    assert_eq!(facts.program, TokenProgramKind::SplToken2022);
    assert_eq!(
        facts.permanent_delegate.as_deref(),
        Some("2apBGMsS6ti9RyF5TwQTDswXBWskiJP2LD4cUEDqYJjk")
    );
    let transfer_fee = facts.transfer_fee.as_ref().expect("fee config present");
    assert_eq!(transfer_fee.basis_points, 0);
    assert!(transfer_fee.authority_present);
    // Hook extension present, no hook program installed.
    assert_eq!(facts.transfer_hook, Some(None));
    assert!(facts.mint_close_authority);
    assert!(facts.confidential_transfers);
    assert_eq!(facts.name.as_deref(), Some("PayPal USD"));
    assert_eq!(facts.symbol.as_deref(), Some("PYUSD"));
    // Every extension in the fixture is either understood or informational.
    assert!(
        facts.other_extensions.is_empty(),
        "got: {:?}",
        facts.other_extensions
    );
}

#[test]
fn pyusd_scores_red_because_of_the_permanent_delegate() {
    let account = fetch_parsed_account(MINT_PYUSD_FIXTURE, PYUSD_MINT);
    let facts = parse_mint_facts(PYUSD_MINT, &account).unwrap();
    let assessment = assess_mint_risk(&facts, None);
    assert_eq!(assessment.level, RiskLevel::Red);
    assert!(assessment.reasons[0].contains("permanent delegate"));
    // The dormant hook and the raisable fee both surface as findings.
    assert!(assessment
        .reasons
        .iter()
        .any(|reason| reason.contains("transfer hook")));
    assert!(assessment
        .reasons
        .iter()
        .any(|reason| reason.contains("fee authority can raise")));
}

#[test]
fn a_token_account_is_rejected_as_not_a_mint() {
    let token_account = ParsedAccountInfo {
        owner_program: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
        parsed_program_label: "spl-token".to_string(),
        parsed_json: json!({ "type": "account", "info": {} }),
    };
    match parse_mint_facts(USDC_MINT, &token_account) {
        Err(CoreError::NotAMint(message)) => assert!(message.contains("type 'account'")),
        other => panic!("expected NotAMint, got {other:?}"),
    }
}

#[test]
fn a_non_token_program_account_is_rejected_as_not_a_mint() {
    let system_account = ParsedAccountInfo {
        owner_program: "11111111111111111111111111111111".to_string(),
        parsed_program_label: "system".to_string(),
        parsed_json: json!({ "type": "account", "info": {} }),
    };
    assert!(matches!(
        parse_mint_facts(USDC_MINT, &system_account),
        Err(CoreError::NotAMint(_))
    ));
}

#[test]
fn a_bare_mint_with_no_authorities_scores_green() {
    let renounced_mint = ParsedAccountInfo {
        owner_program: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
        parsed_program_label: "spl-token".to_string(),
        parsed_json: json!({
            "type": "mint",
            "info": {
                "decimals": 9,
                "supply": "1000000000",
                "isInitialized": true,
                "mintAuthority": null,
                "freezeAuthority": null
            }
        }),
    };
    let facts = parse_mint_facts(USDC_MINT, &renounced_mint).unwrap();
    let assessment = assess_mint_risk(&facts, None);
    assert_eq!(assessment.level, RiskLevel::Green);
    assert!(assessment.reasons.is_empty());
}

#[test]
fn holder_concentration_uses_integer_basis_points() {
    let largest_accounts = vec![
        LargestTokenAccount {
            address: "a1".to_string(),
            amount_base_units: 600,
        },
        LargestTokenAccount {
            address: "a2".to_string(),
            amount_base_units: 200,
        },
        LargestTokenAccount {
            address: "a3".to_string(),
            amount_base_units: 100,
        },
    ];
    let concentration = compute_holder_concentration(&largest_accounts, 1_000).unwrap();
    assert_eq!(concentration.top1_basis_points, 6_000);
    assert_eq!(concentration.top5_basis_points, 9_000);
    assert_eq!(concentration.reported_accounts, 3);
}

#[test]
fn concentration_is_undefined_for_zero_supply_or_no_accounts() {
    assert!(compute_holder_concentration(&[], 1_000).is_none());
    let one_account = vec![LargestTokenAccount {
        address: "a1".to_string(),
        amount_base_units: 1,
    }];
    assert!(compute_holder_concentration(&one_account, 0).is_none());
}

#[test]
fn heavy_concentration_raises_an_amber_finding() {
    let renounced_mint = ParsedAccountInfo {
        owner_program: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
        parsed_program_label: "spl-token".to_string(),
        parsed_json: json!({
            "type": "mint",
            "info": { "decimals": 9, "supply": "1000", "isInitialized": true,
                      "mintAuthority": null, "freezeAuthority": null }
        }),
    };
    let facts = parse_mint_facts(USDC_MINT, &renounced_mint).unwrap();
    let largest_accounts = vec![LargestTokenAccount {
        address: "whale".to_string(),
        amount_base_units: 600,
    }];
    let concentration = compute_holder_concentration(&largest_accounts, facts.supply_base_units);
    let assessment = assess_mint_risk(&facts, concentration.as_ref());
    assert_eq!(assessment.level, RiskLevel::Amber);
    assert!(assessment.reasons[0].contains("60% of supply"));
}
