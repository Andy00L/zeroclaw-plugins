//! Host tests for the JSON-RPC client. Every response is a canned fixture
//! captured verbatim from mainnet on 2026-07-21 (tests/fixtures/), except the
//! getTokenLargestAccounts success which the public RPC rate-limited during
//! capture; that one is synthesized to the documented shape (sourceRef:
//! https://solana.com/docs/rpc/http/gettokenlargestaccounts).

mod common;

use common::MockTransport;
use serde_json::json;
use solana_wasip2_core::addresses::parse_pubkey;
use solana_wasip2_core::error::CoreError;
use solana_wasip2_core::rpc::RpcClient;

const LATEST_BLOCKHASH_FIXTURE: &str = include_str!("fixtures/latest_blockhash.json");
const MINT_USDC_FIXTURE: &str = include_str!("fixtures/mint_usdc_json_parsed.json");
const ACCOUNT_MISSING_FIXTURE: &str = include_str!("fixtures/account_missing.json");
const RPC_ERROR_429_FIXTURE: &str = include_str!("fixtures/rpc_error_429.json");

const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const RPC_URL: &str = "https://rpc.test.invalid";

#[test]
fn latest_blockhash_is_parsed_and_the_request_is_well_formed() {
    let transport = MockTransport::from_fixtures(&[LATEST_BLOCKHASH_FIXTURE]);
    let client = RpcClient::new(transport, RPC_URL);
    let blockhash = client.get_latest_blockhash().unwrap();
    // Value captured in the fixture.
    assert_eq!(blockhash.to_string(), "D277KYCrJsSujJyqKpwwaGW2v8QRFtYnJ3qAC39SZ1tF");
}

#[test]
fn requests_carry_the_jsonrpc_envelope() {
    let transport = MockTransport::from_fixtures(&[LATEST_BLOCKHASH_FIXTURE]);
    {
        let client = RpcClient::new(&transport, RPC_URL);
        client.get_latest_blockhash().unwrap();
    }
    let recorded_requests = transport.recorded_requests.borrow();
    assert_eq!(recorded_requests.len(), 1);
    assert_eq!(recorded_requests[0]["jsonrpc"], "2.0");
    assert_eq!(recorded_requests[0]["method"], "getLatestBlockhash");
}

#[test]
fn a_json_rpc_error_maps_to_a_distinct_error_variant() {
    let transport = MockTransport::from_fixtures(&[RPC_ERROR_429_FIXTURE]);
    let client = RpcClient::new(transport, RPC_URL);
    let call_error = client.get_latest_blockhash().unwrap_err();
    match call_error {
        CoreError::RpcError { code, message } => {
            assert_eq!(code, 429);
            assert!(message.contains("Too many requests"));
        }
        other => panic!("expected RpcError, got {other:?}"),
    }
}

#[test]
fn transport_errors_pass_through_unchanged() {
    let transport = MockTransport::with_responses(vec![Err(CoreError::HttpStatus(500))]);
    let client = RpcClient::new(transport, RPC_URL);
    assert_eq!(client.get_latest_blockhash().unwrap_err(), CoreError::HttpStatus(500));
}

#[test]
fn a_missing_account_is_none_not_an_error() {
    let transport = MockTransport::from_fixtures(&[ACCOUNT_MISSING_FIXTURE]);
    let client = RpcClient::new(transport, RPC_URL);
    let missing_address = parse_pubkey("9PhSoeYzLagajautCYUfUXSB6acpeP1LLQDKpZnegLDq").unwrap();
    assert!(client.get_parsed_account_info(&missing_address).unwrap().is_none());
}

#[test]
fn a_parsed_token_mint_account_is_returned_with_its_owner() {
    let transport = MockTransport::from_fixtures(&[MINT_USDC_FIXTURE]);
    let client = RpcClient::new(transport, RPC_URL);
    let usdc_mint = parse_pubkey(USDC_MINT).unwrap();
    let account = client.get_parsed_account_info(&usdc_mint).unwrap().unwrap();
    assert_eq!(account.owner_program, "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
    assert_eq!(account.parsed_program_label, "spl-token");
    assert_eq!(account.parsed_json["type"], "mint");
}

#[test]
fn account_existence_probe_distinguishes_present_from_absent() {
    let transport = MockTransport::from_fixtures(&[MINT_USDC_FIXTURE, ACCOUNT_MISSING_FIXTURE]);
    let client = RpcClient::new(transport, RPC_URL);
    let usdc_mint = parse_pubkey(USDC_MINT).unwrap();
    assert!(client.account_exists(&usdc_mint).unwrap());
    assert!(!client.account_exists(&usdc_mint).unwrap());
}

#[test]
fn largest_accounts_parse_amounts_as_integers() {
    // Shape per https://solana.com/docs/rpc/http/gettokenlargestaccounts.
    let synthesized_response = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "context": { "apiVersion": "4.1.0", "slot": 434380507 },
            "value": [
                { "address": "3emsAVdmGKERbHjmGfQ6oZ1e35dkf5iYcS6U4CPKFVaa",
                  "amount": "600000000000000", "decimals": 6,
                  "uiAmount": 600000000.0, "uiAmountString": "600000000" },
                { "address": "FGETo8T8wMcN2wCjav8VK6eh3dLk63evNDPxzLSJra8B",
                  "amount": "76942446706090", "decimals": 6,
                  "uiAmount": 76942446.70609, "uiAmountString": "76942446.70609" }
            ]
        }
    });
    let transport = MockTransport::with_responses(vec![Ok(synthesized_response)]);
    let client = RpcClient::new(transport, RPC_URL);
    let mint = parse_pubkey(USDC_MINT).unwrap();
    let largest_accounts = client.get_token_largest_accounts(&mint).unwrap();
    assert_eq!(largest_accounts.len(), 2);
    assert_eq!(largest_accounts[0].amount_base_units, 600_000_000_000_000);
    assert_eq!(largest_accounts[1].address, "FGETo8T8wMcN2wCjav8VK6eh3dLk63evNDPxzLSJra8B");
}

#[test]
fn a_largest_accounts_entry_without_an_amount_is_malformed() {
    let broken_response = json!({
        "jsonrpc": "2.0", "id": 1,
        "result": { "context": {}, "value": [ { "address": "abc" } ] }
    });
    let transport = MockTransport::with_responses(vec![Ok(broken_response)]);
    let client = RpcClient::new(transport, RPC_URL);
    let mint = parse_pubkey(USDC_MINT).unwrap();
    assert!(matches!(
        client.get_token_largest_accounts(&mint),
        Err(CoreError::MalformedResponse(_))
    ));
}
