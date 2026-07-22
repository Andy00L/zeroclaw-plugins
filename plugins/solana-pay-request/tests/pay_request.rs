//! Host tests for the solana-pay-request core. No network exists in this
//! plugin at all; tests inject fixed reference entropy for determinism.

use serde_json::json;
use solana_pay_request::pay_request::{execute_pay_request, ToolOutcome};

/// Operator wallet used across tests (a real mainnet address).
const OPERATOR_WALLET: &str = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
/// Mainnet USDC mint (sourceRef: Circle,
/// https://developers.circle.com/stablecoins/usdc-on-main-networks).
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
/// Base58 of a 32-byte array of ones, the fixed test reference entropy.
const FIXED_REFERENCE: &str = "4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi";

fn fixed_reference_bytes() -> [u8; 32] {
    [1u8; 32]
}

fn run(args: serde_json::Value) -> ToolOutcome {
    execute_pay_request(&args.to_string(), fixed_reference_bytes)
}

fn operator_config() -> serde_json::Value {
    json!({ "recipient": OPERATOR_WALLET })
}

#[test]
fn a_usdc_charge_produces_the_expected_url_and_summary() {
    let outcome = run(json!({
        "amount": "25",
        "label": "Table 4",
        "memo": "order#412",
        "__config": operator_config()
    }));
    assert!(outcome.success, "got: {outcome:?}");
    assert!(
        outcome.output.contains(&format!(
            "Pay URL: solana:{OPERATOR_WALLET}?amount=25&spl-token={USDC_MINT}\
         &reference={FIXED_REFERENCE}&label=Table%204&memo=order%23412"
        )),
        "got: {}",
        outcome.output
    );
    assert!(outcome.output.contains("Amount: 25 USDC"));
    assert!(outcome
        .output
        .contains(&format!("Reference: {FIXED_REFERENCE}")));
}

#[test]
fn sol_requests_omit_the_spl_token_parameter() {
    let outcome = run(json!({
        "amount": "0.50",
        "token": "sol",
        "__config": operator_config()
    }));
    assert!(outcome.success);
    // Canonicalized amount, no spl-token param for native SOL.
    assert!(outcome.output.contains(&format!(
        "Pay URL: solana:{OPERATOR_WALLET}?amount=0.5&reference={FIXED_REFERENCE}"
    )));
    assert!(!outcome.output.contains("spl-token"));
    assert!(outcome.output.contains("Amount: 0.5 SOL"));
}

#[test]
fn without_a_configured_recipient_every_request_fails_closed() {
    let outcome = run(json!({ "amount": "25" }));
    assert!(!outcome.success);
    assert!(outcome.error.unwrap().contains("no recipient configured"));
}

#[test]
fn the_model_cannot_supply_a_recipient_argument() {
    // A prompt-injected model trying to redirect the payment gets a loud
    // error, not a silent ignore.
    let outcome = run(json!({
        "amount": "25",
        "recipient": "attacker1111111111111111111111111111111111111",
        "__config": operator_config()
    }));
    assert!(!outcome.success);
    let error_message = outcome.error.unwrap();
    assert!(
        error_message.contains("invalid arguments"),
        "got: {error_message}"
    );
    assert!(error_message.contains("recipient"), "got: {error_message}");
}

#[test]
fn unknown_token_symbols_fail_closed_and_list_what_is_configured() {
    let outcome = run(json!({
        "amount": "25",
        "token": "DEFINITELY_FAKE",
        "__config": operator_config()
    }));
    assert!(!outcome.success);
    let error_message = outcome.error.unwrap();
    assert!(error_message.contains("'DEFINITELY_FAKE' is not configured"));
    assert!(error_message.contains("SOL"));
    assert!(error_message.contains("USDC"));
}

#[test]
fn operators_can_extend_the_token_map() {
    let outcome = run(json!({
        "amount": "10",
        "token": "pyusd",
        "__config": {
            "recipient": OPERATOR_WALLET,
            "tokens": "PYUSD=2b1kV6DkPAnxd5ixfnxCpjxmKwqjjaYmCZfHsFu24GXo:6"
        }
    }));
    assert!(outcome.success, "got: {outcome:?}");
    assert!(outcome
        .output
        .contains("spl-token=2b1kV6DkPAnxd5ixfnxCpjxmKwqjjaYmCZfHsFu24GXo"));
    assert!(outcome.output.contains("Amount: 10 PYUSD"));
}

#[test]
fn broken_token_config_entries_fail_with_distinct_messages() {
    for (broken_tokens_value, expected_fragment) in [
        ("PYUSD", "must look like SYMBOL=MINT:DECIMALS"),
        (
            "PYUSD=2b1kV6DkPAnxd5ixfnxCpjxmKwqjjaYmCZfHsFu24GXo",
            "missing ':DECIMALS'",
        ),
        ("PYUSD=notbase58:6", "not a valid address"),
        (
            "PYUSD=2b1kV6DkPAnxd5ixfnxCpjxmKwqjjaYmCZfHsFu24GXo:abc",
            "not a number",
        ),
        (
            "PYUSD=2b1kV6DkPAnxd5ixfnxCpjxmKwqjjaYmCZfHsFu24GXo:12",
            "maximum of 9",
        ),
        (
            "=2b1kV6DkPAnxd5ixfnxCpjxmKwqjjaYmCZfHsFu24GXo:6",
            "empty symbol",
        ),
    ] {
        let outcome = run(json!({
            "amount": "1",
            "__config": { "recipient": OPERATOR_WALLET, "tokens": broken_tokens_value }
        }));
        assert!(
            !outcome.success,
            "tokens '{broken_tokens_value}' should fail"
        );
        let error_message = outcome.error.unwrap();
        assert!(
            error_message.contains(expected_fragment),
            "tokens '{broken_tokens_value}': got '{error_message}'"
        );
    }
}

#[test]
fn amounts_are_validated_against_the_tokens_decimals() {
    let outcome = run(json!({
        "amount": "1.1234567",
        "__config": operator_config()
    }));
    assert!(!outcome.success);
    assert!(outcome.error.unwrap().contains("6 decimals"));

    for zero_or_bad_amount in ["0", "-5", "abc", ""] {
        let outcome = run(json!({
            "amount": zero_or_bad_amount,
            "__config": operator_config()
        }));
        assert!(
            !outcome.success,
            "amount '{zero_or_bad_amount}' should fail"
        );
    }
}

#[test]
fn hostile_text_fields_cannot_inject_url_parameters() {
    let outcome = run(json!({
        "amount": "1",
        "label": "shop&amount=999999&recipient=attacker",
        "message": "line one\nline two",
        "__config": operator_config()
    }));
    assert!(outcome.success);
    let url_line = outcome
        .output
        .lines()
        .find(|output_line| output_line.starts_with("Pay URL: "))
        .expect("output must contain the URL line");
    // Exactly one amount parameter, and the injected text arrives encoded.
    assert_eq!(url_line.matches("amount=").count(), 1, "got: {url_line}");
    assert!(url_line.contains("label=shop%26amount%3D999999%26recipient%3Dattacker"));
    // Newlines in the message were flattened before encoding.
    assert!(url_line.contains("message=line%20one%20line%20two"));
}

#[test]
fn token_symbols_tolerate_case_and_surrounding_whitespace() {
    let outcome = run(json!({
        "amount": "1",
        "token": "  usdc  ",
        "__config": operator_config()
    }));
    assert!(outcome.success, "got: {outcome:?}");
    assert!(outcome.output.contains("Amount: 1 USDC"));
}

#[test]
fn text_fields_that_sanitize_to_nothing_are_omitted() {
    // A label of pure whitespace and control characters must not become an
    // empty `label=` parameter in the URL.
    let outcome = run(json!({
        "amount": "1",
        "label": "\n\t  \r",
        "__config": operator_config()
    }));
    assert!(outcome.success);
    assert!(
        !outcome.output.contains("label="),
        "got: {}",
        outcome.output
    );
}

#[test]
fn long_labels_are_truncated_before_they_reach_the_url() {
    let outcome = run(json!({
        "amount": "1",
        "label": "A".repeat(200),
        "__config": operator_config()
    }));
    assert!(outcome.success);
    // 64 characters survive plus the ".." truncation marker; dots are RFC
    // 3986 unreserved so the marker stays literal in the URL.
    let expected_label = format!("label={}..", "A".repeat(64));
    assert!(
        outcome.output.contains(&expected_label),
        "got: {}",
        outcome.output
    );
    assert!(!outcome.output.contains(&"A".repeat(65)));
}

#[test]
fn a_numeric_amount_is_rejected_as_a_type_error() {
    // The schema says string; a bare JSON number must fail loudly, not be
    // coerced (float coercion is how rounding bugs enter money paths).
    let outcome = run(json!({
        "amount": 25,
        "__config": operator_config()
    }));
    assert!(!outcome.success);
    assert!(outcome.error.unwrap().contains("invalid arguments"));
}

#[test]
fn operators_may_redefine_the_built_in_usdc_entry() {
    // Config is operator-trusted: redefining USDC (for example to a devnet
    // mint) is intentional and the last definition wins.
    let outcome = run(json!({
        "amount": "10",
        "token": "USDC",
        "__config": {
            "recipient": OPERATOR_WALLET,
            "tokens": "USDC=2b1kV6DkPAnxd5ixfnxCpjxmKwqjjaYmCZfHsFu24GXo:6"
        }
    }));
    assert!(outcome.success, "got: {outcome:?}");
    assert!(outcome
        .output
        .contains("spl-token=2b1kV6DkPAnxd5ixfnxCpjxmKwqjjaYmCZfHsFu24GXo"));
}

#[test]
fn an_invalid_configured_recipient_is_a_config_error() {
    let outcome = run(json!({
        "amount": "1",
        "__config": { "recipient": "not-an-address" }
    }));
    assert!(!outcome.success);
    assert!(outcome.error.unwrap().contains("config error: recipient"));
}

#[test]
fn a_typoed_config_key_refuses_to_run() {
    let outcome = run(json!({
        "amount": "1",
        "__config": { "recipient": OPERATOR_WALLET, "recipiend_backup": "typo" }
    }));
    assert!(!outcome.success);
    let error_message = outcome.error.unwrap();
    assert!(
        error_message.contains("unknown key(s) recipiend_backup"),
        "got: {error_message}"
    );
    assert!(error_message.contains("accepted keys: recipient, tokens"));
}
