//! Host tests for the payment-watch core. Fixtures are real mainnet
//! captures: a signature list whose newest entry genuinely failed, and a
//! USDC transfer whose recipient delta (+1.999740) differs from the sent
//! amount (2.000000) because a fee-splitter took 260 base units.

use std::cell::RefCell;
use std::collections::VecDeque;

use payment_watch::payment_watch::{execute_payment_watch, ToolOutcome};
use serde_json::{json, Value};
use solana_wasip2_core::error::CoreError;
use solana_wasip2_core::http::JsonHttpTransport;

const SIGNATURES_FIXTURE: &str = include_str!("fixtures/signatures_for_address.json");
const TRANSACTION_FIXTURE: &str = include_str!("fixtures/transaction_usdc_transfer.json");

/// The wallet that actually received +1.999740 USDC in the captured
/// transaction; used as the operator recipient so the fixture is ground
/// truth, not a synthetic story.
const RECEIVING_WALLET: &str = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
/// Any valid base58 address works as the watched reference in tests.
const REFERENCE: &str = "2apBGMsS6ti9RyF5TwQTDswXBWskiJP2LD4cUEDqYJjk";
/// The successful signature inside the signatures fixture.
const SETTLING_SIGNATURE: &str =
    "8gXefqe2LCKPP5YNUuBrZEAWHDcVP3GrHjvcnN2U1c9UbBUXnxzNNSmJ9ws6Re2UBm6XM2fhX2SeJrz2SNo9DiC";

struct MockTransport {
    queued_responses: RefCell<VecDeque<Value>>,
    recorded_requests: RefCell<Vec<Value>>,
}

impl MockTransport {
    fn from_values(response_values: Vec<Value>) -> Self {
        Self {
            queued_responses: RefCell::new(response_values.into()),
            recorded_requests: RefCell::new(Vec::new()),
        }
    }
}

impl JsonHttpTransport for MockTransport {
    fn post_json(&self, _url: &str, body: &Value) -> Result<Value, CoreError> {
        self.recorded_requests.borrow_mut().push(body.clone());
        self.queued_responses
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| CoreError::TransportFailed("mock: no queued response".to_string()))
    }
}

fn fixture(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn null_transaction_response() -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "result": null })
}

fn watch_config() -> Value {
    json!({ "recipient": RECEIVING_WALLET })
}

fn run(transport: &MockTransport, args: Value) -> ToolOutcome {
    execute_payment_watch(transport, &args.to_string())
}

/// The standard fixture queue: 3 signatures (1 failed), the real transfer
/// for the first successful signature, and "not yet queryable" for the
/// second.
fn standard_queue() -> Vec<Value> {
    vec![
        fixture(SIGNATURES_FIXTURE),
        fixture(TRANSACTION_FIXTURE),
        null_transaction_response(),
    ]
}

#[test]
fn an_exactly_settled_invoice_reports_paid_with_evidence() {
    let transport = MockTransport::from_values(standard_queue());
    let outcome = run(
        &transport,
        json!({ "reference": REFERENCE, "amount": "1.99974", "__config": watch_config() }),
    );
    assert!(outcome.success, "got: {outcome:?}");
    assert!(outcome.output.starts_with("Payment status: PAID"));
    assert!(outcome
        .output
        .contains("Received: 1.99974 USDC across 1 settling transaction(s)"));
    assert!(outcome.output.contains(SETTLING_SIGNATURE));
    assert!(outcome.output.contains("skipped 1 failed transaction(s)"));
    assert!(outcome
        .output
        .contains("1 transaction(s) not yet queryable"));
    // The newest signature (the failed one) becomes the cursor.
    assert!(outcome.output.contains("Cursor: 2ap2o5LHSQVT8LbxVyPa"));
}

#[test]
fn an_underpaid_invoice_reports_partial_not_paid() {
    let transport = MockTransport::from_values(standard_queue());
    let outcome = run(
        &transport,
        json!({ "reference": REFERENCE, "amount": "2", "__config": watch_config() }),
    );
    assert!(outcome.success);
    assert!(outcome.output.starts_with("Payment status: PARTIAL"));
    assert!(outcome.output.contains("Expected: 2 USDC"));
    assert!(outcome.output.contains("Received: 1.99974 USDC"));
}

#[test]
fn an_overpaid_invoice_reports_paid_with_the_surplus() {
    let transport = MockTransport::from_values(standard_queue());
    let outcome = run(
        &transport,
        json!({ "reference": REFERENCE, "amount": "1.5", "__config": watch_config() }),
    );
    assert!(outcome.success);
    assert!(outcome.output.starts_with("Payment status: PAID"));
    assert!(outcome.output.contains("overpaid by 0.49974 USDC"));
}

#[test]
fn no_transactions_means_pending_and_no_further_rpc_calls() {
    let transport = MockTransport::from_values(vec![json!({
        "jsonrpc": "2.0", "id": 1, "result": []
    })]);
    let outcome = run(
        &transport,
        json!({ "reference": REFERENCE, "amount": "25", "__config": watch_config() }),
    );
    assert!(outcome.success);
    assert!(outcome.output.starts_with("Payment status: PENDING"));
    assert_eq!(transport.recorded_requests.borrow().len(), 1);
}

#[test]
fn a_reference_touch_without_value_is_called_out_not_counted() {
    // The settling transaction pays someone else: the configured recipient
    // is not among the token balance owners, so the delta is zero.
    let transport = MockTransport::from_values(standard_queue());
    let outcome = run(
        &transport,
        json!({
            "reference": REFERENCE, "amount": "1",
            "__config": { "recipient": "9PhSoeYzLagajautCYUfUXSB6acpeP1LLQDKpZnegLDq" }
        }),
    );
    assert!(outcome.success);
    assert!(outcome.output.starts_with("Payment status: PENDING"));
    assert!(outcome
        .output
        .contains("a reference touch is not a payment"));
}

#[test]
fn the_cursor_is_validated_and_forwarded() {
    let transport = MockTransport::from_values(vec![json!({
        "jsonrpc": "2.0", "id": 1, "result": []
    })]);
    let outcome = run(
        &transport,
        json!({
            "reference": REFERENCE, "amount": "25",
            "cursor": SETTLING_SIGNATURE,
            "__config": watch_config()
        }),
    );
    assert!(outcome.success);
    assert!(outcome
        .output
        .contains(&format!("Cursor: {SETTLING_SIGNATURE} (unchanged")));
    let recorded_requests = transport.recorded_requests.borrow();
    assert_eq!(
        recorded_requests[0]["params"][1]["until"],
        SETTLING_SIGNATURE
    );
}

#[test]
fn a_malformed_cursor_fails_before_any_rpc_call() {
    let transport = MockTransport::from_values(vec![]);
    let outcome = run(
        &transport,
        json!({
            "reference": REFERENCE, "amount": "25",
            "cursor": "'; drop table--",
            "__config": watch_config()
        }),
    );
    assert!(!outcome.success);
    assert!(outcome
        .error
        .unwrap()
        .contains("not a base58 transaction signature"));
    assert!(transport.recorded_requests.borrow().is_empty());
}

#[test]
fn without_a_configured_recipient_every_check_fails_closed() {
    let transport = MockTransport::from_values(vec![]);
    let outcome = run(
        &transport,
        json!({ "reference": REFERENCE, "amount": "25" }),
    );
    assert!(!outcome.success);
    assert!(outcome.error.unwrap().contains("no recipient configured"));
    assert!(transport.recorded_requests.borrow().is_empty());
}

#[test]
fn the_model_cannot_supply_a_recipient_argument() {
    let transport = MockTransport::from_values(vec![]);
    let outcome = run(
        &transport,
        json!({
            "reference": REFERENCE, "amount": "25",
            "recipient": "attacker-wallet",
            "__config": watch_config()
        }),
    );
    assert!(!outcome.success);
    let error_message = outcome.error.unwrap();
    assert!(error_message.contains("invalid arguments"));
    assert!(error_message.contains("recipient"));
    assert!(transport.recorded_requests.borrow().is_empty());
}

#[test]
fn a_typoed_config_key_refuses_to_run() {
    let transport = MockTransport::from_values(vec![]);
    let outcome = run(
        &transport,
        json!({
            "reference": REFERENCE, "amount": "25",
            "__config": { "recipient": RECEIVING_WALLET, "recipent": "typo" }
        }),
    );
    assert!(!outcome.success);
    assert!(outcome.error.unwrap().contains("unknown key(s) recipent"));
}

#[test]
fn unknown_tokens_and_bad_amounts_fail_closed() {
    let transport = MockTransport::from_values(vec![]);
    let outcome = run(
        &transport,
        json!({
            "reference": REFERENCE, "amount": "25", "token": "FAKECOIN",
            "__config": watch_config()
        }),
    );
    assert!(!outcome.success);
    assert!(outcome
        .error
        .unwrap()
        .contains("'FAKECOIN' is not configured"));

    for bad_amount in ["0", "abc", "-1", ""] {
        let transport = MockTransport::from_values(vec![]);
        let outcome = run(
            &transport,
            json!({ "reference": REFERENCE, "amount": bad_amount, "__config": watch_config() }),
        );
        assert!(!outcome.success, "amount '{bad_amount}' should fail");
    }
}

#[test]
fn the_report_stays_context_window_friendly() {
    let transport = MockTransport::from_values(standard_queue());
    let outcome = run(
        &transport,
        json!({ "reference": REFERENCE, "amount": "1.99974", "__config": watch_config() }),
    );
    assert!(outcome.success);
    assert!(
        outcome.output.len() < 1_000,
        "report too long: {} chars",
        outcome.output.len()
    );
}
