//! Solana Pay transfer request URL construction.
//!
//! Builds `solana:<recipient>?...` URLs per the Solana Pay spec
//! (sourceRef: https://docs.solanapay.com/spec, Transfer Request). Query
//! values are percent-encoded per RFC 3986; the recipient, spl-token, and
//! reference values are base58 and need no encoding but pass through the
//! same encoder for uniformity.

use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use solana_pubkey::Pubkey;

/// RFC 3986 unreserved characters stay literal; everything else is encoded.
/// sourceRef: RFC 3986 section 2.3 (unreserved = ALPHA / DIGIT / "-" / "." /
/// "_" / "~").
const QUERY_VALUE_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// A Solana Pay transfer request. `amount` must already be a canonical
/// decimal string (see `amount::canonicalize_decimal_amount`); when `None`
/// the wallet prompts the payer for the amount, per spec.
#[derive(Debug)]
pub struct TransferRequest {
    pub recipient: Pubkey,
    pub amount: Option<String>,
    /// SPL mint for a token transfer; `None` means native SOL.
    pub spl_token: Option<Pubkey>,
    /// Reference keys the wallet must attach as read-only non-signer
    /// accounts, so the payment can be found via getSignaturesForAddress.
    pub reference: Vec<Pubkey>,
    pub label: Option<String>,
    pub message: Option<String>,
    pub memo: Option<String>,
}

/// Render the `solana:` URL. Parameter order follows the spec's field order:
/// amount, spl-token, reference, label, message, memo.
pub fn build_transfer_request_url(request: &TransferRequest) -> String {
    let mut query_parts: Vec<String> = Vec::new();
    if let Some(amount_text) = &request.amount {
        query_parts.push(format!(
            "amount={}",
            utf8_percent_encode(amount_text, QUERY_VALUE_ENCODE_SET)
        ));
    }
    if let Some(spl_token_mint) = &request.spl_token {
        query_parts.push(format!("spl-token={spl_token_mint}"));
    }
    for reference_key in &request.reference {
        query_parts.push(format!("reference={reference_key}"));
    }
    if let Some(label_text) = &request.label {
        query_parts.push(format!(
            "label={}",
            utf8_percent_encode(label_text, QUERY_VALUE_ENCODE_SET)
        ));
    }
    if let Some(message_text) = &request.message {
        query_parts.push(format!(
            "message={}",
            utf8_percent_encode(message_text, QUERY_VALUE_ENCODE_SET)
        ));
    }
    if let Some(memo_text) = &request.memo {
        query_parts.push(format!(
            "memo={}",
            utf8_percent_encode(memo_text, QUERY_VALUE_ENCODE_SET)
        ));
    }

    if query_parts.is_empty() {
        format!("solana:{}", request.recipient)
    } else {
        format!("solana:{}?{}", request.recipient, query_parts.join("&"))
    }
}
