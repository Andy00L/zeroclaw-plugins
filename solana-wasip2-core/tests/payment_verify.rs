//! Host tests for settlement verification, driven end to end through the
//! RPC client with fixtures captured verbatim from mainnet on 2026-07-21:
//! a real USDC transfer (sender -2.000000, recipient +1.999740, a
//! fee-splitter +0.000260) and a signature list containing a genuinely
//! failed transaction.

mod common;

use common::MockTransport;
use serde_json::json;
use solana_wasip2_core::addresses::parse_pubkey;
use solana_wasip2_core::error::CoreError;
use solana_wasip2_core::payment_verify::{
    compute_recipient_lamport_delta, compute_recipient_token_delta, transaction_failed,
};
use solana_wasip2_core::rpc::RpcClient;

const SIGNATURES_FIXTURE: &str = include_str!("fixtures/signatures_for_address.json");
const TRANSACTION_FIXTURE: &str = include_str!("fixtures/transaction_usdc_transfer.json");

const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
/// Parties observed in the captured transaction.
const RECEIVING_WALLET: &str = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
const SENDING_WALLET: &str = "E1bQJ8eMMn3zmeSewW3HQ8zmJr7KR75JonbwAtWx2bux";
const FEE_PAYER_WALLET: &str = "Edyca9eoBbceGU6UUHxC78o8W3cLEZyuYaA8wYnXxmgP";
const RPC_URL: &str = "https://rpc.test.invalid";

fn fixture_transaction() -> serde_json::Value {
    let envelope: serde_json::Value = serde_json::from_str(TRANSACTION_FIXTURE).unwrap();
    envelope["result"].clone()
}

#[test]
fn signature_records_carry_the_failure_flag() {
    let transport = MockTransport::from_fixtures(&[SIGNATURES_FIXTURE]);
    let client = RpcClient::new(transport, RPC_URL);
    let watched_address = parse_pubkey("FGETo8T8wMcN2wCjav8VK6eh3dLk63evNDPxzLSJra8B").unwrap();
    let records = client
        .get_signatures_for_address(&watched_address, None, 3)
        .unwrap();
    assert_eq!(records.len(), 3);
    // The newest captured transaction genuinely failed on mainnet.
    assert!(records[0].failed);
    assert!(!records[1].failed);
    assert!(!records[2].failed);
}

#[test]
fn the_until_cursor_is_forwarded_in_the_request() {
    let transport = MockTransport::from_fixtures(&[SIGNATURES_FIXTURE]);
    let watched_address = parse_pubkey("FGETo8T8wMcN2wCjav8VK6eh3dLk63evNDPxzLSJra8B").unwrap();
    {
        let client = RpcClient::new(&transport, RPC_URL);
        client
            .get_signatures_for_address(&watched_address, Some("cursorSig111"), 5)
            .unwrap();
    }
    let recorded_requests = transport.recorded_requests.borrow();
    assert_eq!(recorded_requests[0]["params"][1]["until"], "cursorSig111");
    assert_eq!(recorded_requests[0]["params"][1]["limit"], 5);
}

#[test]
fn get_transaction_returns_none_for_unknown_signatures() {
    let transport = MockTransport::with_responses(vec![Ok(
        json!({ "jsonrpc": "2.0", "id": 1, "result": null }),
    )]);
    let client = RpcClient::new(transport, RPC_URL);
    assert!(client.get_transaction_json("unknownSig").unwrap().is_none());
}

#[test]
fn token_deltas_match_the_real_transfer() {
    let transaction = fixture_transaction();
    assert!(!transaction_failed(&transaction));
    // Captured ground truth: +1.999740 to the recipient, -2.000000 from the
    // sender (a 260-base-unit fee was split to a third account).
    assert_eq!(
        compute_recipient_token_delta(&transaction, RECEIVING_WALLET, USDC_MINT).unwrap(),
        1_999_740
    );
    assert_eq!(
        compute_recipient_token_delta(&transaction, SENDING_WALLET, USDC_MINT).unwrap(),
        -2_000_000
    );
}

#[test]
fn unrelated_owners_and_mints_have_zero_delta() {
    let transaction = fixture_transaction();
    assert_eq!(
        compute_recipient_token_delta(&transaction, FEE_PAYER_WALLET, USDC_MINT).unwrap(),
        0
    );
    assert_eq!(
        compute_recipient_token_delta(
            &transaction,
            RECEIVING_WALLET,
            "2b1kV6DkPAnxd5ixfnxCpjxmKwqjjaYmCZfHsFu24GXo"
        )
        .unwrap(),
        0
    );
}

#[test]
fn lamport_deltas_read_the_balance_arrays() {
    let transaction = fixture_transaction();
    // The fee payer lost at least the 5000-lamport fee.
    let fee_payer_delta = compute_recipient_lamport_delta(&transaction, FEE_PAYER_WALLET).unwrap();
    assert!(fee_payer_delta <= -5_000, "got: {fee_payer_delta}");
    // A wallet absent from the account keys has no delta.
    assert_eq!(
        compute_recipient_lamport_delta(
            &transaction,
            "9PhSoeYzLagajautCYUfUXSB6acpeP1LLQDKpZnegLDq"
        )
        .unwrap(),
        0
    );
}

#[test]
fn multiple_token_accounts_for_one_owner_are_summed_per_side() {
    // An owner can hold the same mint in several token accounts inside one
    // transaction; the delta is the sum over all of them, per side.
    let transaction = json!({
        "meta": {
            "err": null,
            "preTokenBalances": [
                { "owner": "wallet", "mint": "mint", "uiTokenAmount": { "amount": "100" } },
                { "owner": "wallet", "mint": "mint", "uiTokenAmount": { "amount": "50" } }
            ],
            "postTokenBalances": [
                { "owner": "wallet", "mint": "mint", "uiTokenAmount": { "amount": "200" } },
                { "owner": "wallet", "mint": "mint", "uiTokenAmount": { "amount": "75" } }
            ]
        }
    });
    assert_eq!(
        compute_recipient_token_delta(&transaction, "wallet", "mint").unwrap(),
        125
    );
}

#[test]
fn plain_string_account_keys_are_supported_for_lamport_deltas() {
    // json (non-parsed) encoding lists accountKeys as bare strings instead
    // of {pubkey} objects; both shapes must resolve the balance index.
    let transaction = json!({
        "transaction": { "message": { "accountKeys": ["feePayer111", "recipient111"] } },
        "meta": { "err": null, "preBalances": [10_000, 100], "postBalances": [4_000, 250] }
    });
    assert_eq!(
        compute_recipient_lamport_delta(&transaction, "recipient111").unwrap(),
        150
    );
}

#[test]
fn a_balance_array_shorter_than_the_key_index_is_malformed() {
    let transaction = json!({
        "transaction": { "message": { "accountKeys": ["feePayer111", "recipient111"] } },
        "meta": { "err": null, "preBalances": [10_000, 100], "postBalances": [4_000] }
    });
    match compute_recipient_lamport_delta(&transaction, "recipient111") {
        Err(CoreError::MalformedResponse(message)) => {
            assert!(message.contains("postBalances"), "got: {message}");
        }
        other => panic!("expected MalformedResponse, got {other:?}"),
    }
}

#[test]
fn a_non_string_token_amount_is_malformed_not_zero() {
    // uiTokenAmount.amount is specified as a string; a numeric value means
    // the response is not what the RPC documentation promises, and treating
    // it as zero would silently undercount a payment.
    let transaction = json!({
        "meta": {
            "err": null,
            "preTokenBalances": [],
            "postTokenBalances": [
                { "owner": "wallet", "mint": "mint", "uiTokenAmount": { "amount": 200 } }
            ]
        }
    });
    assert!(matches!(
        compute_recipient_token_delta(&transaction, "wallet", "mint"),
        Err(CoreError::MalformedResponse(_))
    ));
}

#[test]
fn a_failed_transaction_is_flagged() {
    let mut transaction = fixture_transaction();
    transaction["meta"]["err"] = json!({ "InstructionError": [0, { "Custom": 1 }] });
    assert!(transaction_failed(&transaction));
}
