//! Host tests for config-section hygiene: unknown keys are detected so
//! plugins can refuse a mistyped configuration instead of silently running
//! with a guardrail off.

use std::collections::HashMap;

use solana_wasip2_core::config::{describe_unknown_config_keys, find_unknown_config_keys};

fn section_of(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

#[test]
fn a_clean_section_yields_no_unknown_keys() {
    let section = section_of(&[("rpc_url", "https://x"), ("tokens", "A=B:1")]);
    assert!(find_unknown_config_keys(&section, &["rpc_url", "tokens"]).is_empty());
    assert!(find_unknown_config_keys(&HashMap::new(), &["rpc_url"]).is_empty());
}

#[test]
fn typoed_keys_are_detected_and_sorted() {
    let section = section_of(&[
        ("rpc_url", "https://x"),
        ("allowed_recipient", "oops-singular"),
        ("Sender_Wallet", "oops-case"),
    ]);
    let unknown_keys = find_unknown_config_keys(
        &section,
        &["rpc_url", "allowed_recipients", "sender_wallet"],
    );
    assert_eq!(unknown_keys, vec!["Sender_Wallet", "allowed_recipient"]);
}

#[test]
fn the_message_names_both_the_bad_and_the_accepted_keys() {
    let message = describe_unknown_config_keys(
        &["allowed_recipient".to_string()],
        &["allowed_recipients", "rpc_url"],
    );
    assert!(message.contains("unknown key(s) allowed_recipient"));
    assert!(message.contains("accepted keys: allowed_recipients, rpc_url"));
    assert!(message.contains("Refusing to run"));
}
