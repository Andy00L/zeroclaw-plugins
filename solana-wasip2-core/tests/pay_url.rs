//! Host tests for Solana Pay transfer request URLs
//! (spec: https://docs.solanapay.com/spec, Transfer Request).

use solana_wasip2_core::addresses::parse_pubkey;
use solana_wasip2_core::pay_url::{build_transfer_request_url, TransferRequest};

/// Mainnet USDC mint (sourceRef: Circle,
/// https://developers.circle.com/stablecoins/usdc-on-main-networks).
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const RECIPIENT: &str = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
const REFERENCE: &str = "2apBGMsS6ti9RyF5TwQTDswXBWskiJP2LD4cUEDqYJjk";

#[test]
fn builds_a_full_spl_transfer_request() {
    let request = TransferRequest {
        recipient: parse_pubkey(RECIPIENT).unwrap(),
        amount: Some("25".to_string()),
        spl_token: Some(parse_pubkey(USDC_MINT).unwrap()),
        reference: vec![parse_pubkey(REFERENCE).unwrap()],
        label: Some("Table 4".to_string()),
        message: Some("Dinner at Casa Zero".to_string()),
        memo: Some("order#412".to_string()),
    };
    assert_eq!(
        build_transfer_request_url(&request),
        format!(
            "solana:{RECIPIENT}?amount=25&spl-token={USDC_MINT}&reference={REFERENCE}\
             &label=Table%204&message=Dinner%20at%20Casa%20Zero&memo=order%23412"
        )
    );
}

#[test]
fn recipient_alone_yields_a_bare_url() {
    let request = TransferRequest {
        recipient: parse_pubkey(RECIPIENT).unwrap(),
        amount: None,
        spl_token: None,
        reference: Vec::new(),
        label: None,
        message: None,
        memo: None,
    };
    assert_eq!(
        build_transfer_request_url(&request),
        format!("solana:{RECIPIENT}")
    );
}

#[test]
fn encodes_multibyte_utf8_in_labels() {
    let request = TransferRequest {
        recipient: parse_pubkey(RECIPIENT).unwrap(),
        amount: None,
        spl_token: None,
        reference: Vec::new(),
        label: Some("Café ☕".to_string()),
        message: None,
        memo: None,
    };
    assert_eq!(
        build_transfer_request_url(&request),
        format!("solana:{RECIPIENT}?label=Caf%C3%A9%20%E2%98%95")
    );
}

#[test]
fn url_metacharacters_in_text_fields_cannot_inject_parameters() {
    // A hostile label must not be able to smuggle an amount override into
    // the URL: '&' and '=' have to arrive percent-encoded.
    let request = TransferRequest {
        recipient: parse_pubkey(RECIPIENT).unwrap(),
        amount: Some("1".to_string()),
        spl_token: None,
        reference: Vec::new(),
        label: Some("pay&amount=999999".to_string()),
        message: None,
        memo: None,
    };
    let url = build_transfer_request_url(&request);
    assert!(url.contains("label=pay%26amount%3D999999"), "got: {url}");
    assert_eq!(url.matches("&amount=").count(), 0, "got: {url}");
}

#[test]
fn every_rfc3986_reserved_character_arrives_percent_encoded() {
    // '%' itself must encode to %25 or a crafted label could smuggle
    // pre-encoded sequences past a naive decoder.
    let request = TransferRequest {
        recipient: parse_pubkey(RECIPIENT).unwrap(),
        amount: None,
        spl_token: None,
        reference: Vec::new(),
        label: Some("100% off?/#=&+".to_string()),
        message: None,
        memo: None,
    };
    assert_eq!(
        build_transfer_request_url(&request),
        format!("solana:{RECIPIENT}?label=100%25%20off%3F%2F%23%3D%26%2B")
    );
}

#[test]
fn multiple_references_repeat_the_parameter() {
    let request = TransferRequest {
        recipient: parse_pubkey(RECIPIENT).unwrap(),
        amount: None,
        spl_token: None,
        reference: vec![
            parse_pubkey(REFERENCE).unwrap(),
            parse_pubkey(USDC_MINT).unwrap(),
        ],
        label: None,
        message: None,
        memo: None,
    };
    assert_eq!(
        build_transfer_request_url(&request),
        format!("solana:{RECIPIENT}?reference={REFERENCE}&reference={USDC_MINT}")
    );
}
