//! Honest payment settlement verification.
//!
//! A transaction that merely references an address is not a payment: anyone
//! can attach someone's reference key to a worthless transaction. The only
//! honest signal is the recipient's balance delta inside the transaction's
//! metadata: `preTokenBalances`/`postTokenBalances` for SPL tokens and
//! `preBalances`/`postBalances` for native SOL (sourceRef:
//! https://solana.com/docs/rpc/http/gettransaction, meta fields). This
//! module computes those deltas from a `getTransaction` jsonParsed payload.

use serde_json::Value;

use crate::error::CoreError;

/// True when the transaction executed and failed (`meta.err` non-null).
/// Failed transactions move no value regardless of what they reference.
pub fn transaction_failed(transaction_json: &Value) -> bool {
    !matches!(
        transaction_json.pointer("/meta/err"),
        Some(Value::Null) | None
    )
}

/// Sum a token-balance side (pre or post) for one owner and mint, in base
/// units. Multiple token accounts for the same owner are summed.
fn sum_token_balances_for_owner(
    balance_entries: Option<&Value>,
    recipient_wallet: &str,
    mint: &str,
) -> Result<u128, CoreError> {
    let entries = match balance_entries.and_then(Value::as_array) {
        Some(entries) => entries,
        None => return Ok(0),
    };
    let mut total_base_units: u128 = 0;
    for entry in entries {
        let entry_owner = entry.get("owner").and_then(Value::as_str).unwrap_or("");
        let entry_mint = entry.get("mint").and_then(Value::as_str).unwrap_or("");
        if entry_owner != recipient_wallet || entry_mint != mint {
            continue;
        }
        let amount_base_units = entry
            .pointer("/uiTokenAmount/amount")
            .and_then(Value::as_str)
            .and_then(|amount_text| amount_text.parse::<u128>().ok())
            .ok_or_else(|| {
                CoreError::MalformedResponse(
                    "token balance entry missing integer amount".to_string(),
                )
            })?;
        total_base_units += amount_base_units;
    }
    Ok(total_base_units)
}

/// Net change of `recipient_wallet`'s holdings of `mint` inside one
/// transaction, in base units. Positive means the recipient received value.
pub fn compute_recipient_token_delta(
    transaction_json: &Value,
    recipient_wallet: &str,
    mint: &str,
) -> Result<i128, CoreError> {
    let pre_total = sum_token_balances_for_owner(
        transaction_json.pointer("/meta/preTokenBalances"),
        recipient_wallet,
        mint,
    )?;
    let post_total = sum_token_balances_for_owner(
        transaction_json.pointer("/meta/postTokenBalances"),
        recipient_wallet,
        mint,
    )?;
    Ok(post_total as i128 - pre_total as i128)
}

/// Net lamport change of `recipient_wallet` inside one transaction.
/// Returns 0 when the wallet is not among the transaction's account keys.
/// Account keys arrive either as jsonParsed objects with a `pubkey` field
/// or as plain strings; both are handled.
pub fn compute_recipient_lamport_delta(
    transaction_json: &Value,
    recipient_wallet: &str,
) -> Result<i128, CoreError> {
    let account_keys = transaction_json
        .pointer("/transaction/message/accountKeys")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            CoreError::MalformedResponse("transaction missing accountKeys".to_string())
        })?;
    let recipient_index = account_keys.iter().position(|account_key| {
        let key_text = account_key
            .as_str()
            .or_else(|| account_key.get("pubkey").and_then(Value::as_str))
            .unwrap_or("");
        key_text == recipient_wallet
    });
    let Some(recipient_index) = recipient_index else {
        return Ok(0);
    };

    let read_balance_at = |pointer: &str| -> Result<i128, CoreError> {
        transaction_json
            .pointer(pointer)
            .and_then(Value::as_array)
            .and_then(|balances| balances.get(recipient_index))
            .and_then(Value::as_u64)
            .map(i128::from)
            .ok_or_else(|| {
                CoreError::MalformedResponse(format!(
                    "transaction missing balance entry at {pointer}[{recipient_index}]"
                ))
            })
    };
    Ok(read_balance_at("/meta/postBalances")? - read_balance_at("/meta/preBalances")?)
}
