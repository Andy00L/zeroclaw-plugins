//! Minimal Solana JSON-RPC client over a blocking JSON transport.
//!
//! Implements exactly the methods the plugins in this suite need, shaped so
//! callers receive small typed values instead of raw RPC payloads.
//! Method names and response shapes follow the official RPC documentation
//! (sourceRef: https://solana.com/docs/rpc).

use base64::Engine as _;
use serde_json::{json, Value};
use solana_hash::Hash;
use solana_pubkey::Pubkey;

use crate::error::CoreError;
use crate::http::JsonHttpTransport;

/// One parsed account from `getAccountInfo` with `jsonParsed` encoding.
#[derive(Debug)]
pub struct ParsedAccountInfo {
    /// Base58 address of the program owning the account.
    pub owner_program: String,
    /// Parser label reported by the RPC, for example "spl-token-2022".
    pub parsed_program_label: String,
    /// The `data.parsed` JSON subtree (kind-specific).
    pub parsed_json: Value,
}

/// One entry from `getTokenLargestAccounts`.
#[derive(Debug)]
pub struct LargestTokenAccount {
    pub address: String,
    pub amount_base_units: u128,
}

/// One entry from `getSignaturesForAddress`.
#[derive(Debug)]
pub struct SignatureRecord {
    pub signature: String,
    pub slot: u64,
    /// The transaction landed but its execution failed (`err` non-null);
    /// a failed transaction moved no value.
    pub failed: bool,
}

pub struct RpcClient<Transport: JsonHttpTransport> {
    transport: Transport,
    rpc_url: String,
}

impl<Transport: JsonHttpTransport> RpcClient<Transport> {
    pub fn new(transport: Transport, rpc_url: impl Into<String>) -> Self {
        Self {
            transport,
            rpc_url: rpc_url.into(),
        }
    }

    /// POST one JSON-RPC 2.0 call and unwrap the `result` field, converting
    /// a JSON-RPC `error` object into `CoreError::RpcError`.
    fn call(&self, method: &str, params: Value) -> Result<Value, CoreError> {
        let request_body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });
        let response_json = self.transport.post_json(&self.rpc_url, &request_body)?;
        if let Some(error_value) = response_json.get("error") {
            let code = error_value.get("code").and_then(Value::as_i64).unwrap_or(0);
            let message = error_value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("(no message)")
                .to_string();
            return Err(CoreError::RpcError { code, message });
        }
        response_json
            .get("result")
            .cloned()
            .ok_or_else(|| CoreError::MalformedResponse(format!("{method}: missing result field")))
    }

    /// `getLatestBlockhash` at confirmed commitment.
    pub fn get_latest_blockhash(&self) -> Result<Hash, CoreError> {
        let result = self.call("getLatestBlockhash", json!([{ "commitment": "confirmed" }]))?;
        let blockhash_text = result
            .pointer("/value/blockhash")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                CoreError::MalformedResponse(
                    "getLatestBlockhash: missing value.blockhash".to_string(),
                )
            })?;
        blockhash_text
            .parse::<Hash>()
            .map_err(|_| CoreError::InvalidBlockhash(blockhash_text.to_string()))
    }

    /// `getAccountInfo` with `jsonParsed` encoding. `Ok(None)` means the
    /// account does not exist (RPC `value: null`).
    pub fn get_parsed_account_info(
        &self,
        address: &Pubkey,
    ) -> Result<Option<ParsedAccountInfo>, CoreError> {
        let result = self.call(
            "getAccountInfo",
            json!([address.to_string(), { "encoding": "jsonParsed" }]),
        )?;
        let account_value = match result.get("value") {
            Some(Value::Null) | None => return Ok(None),
            Some(account_value) => account_value,
        };
        let owner_program = account_value
            .get("owner")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                CoreError::MalformedResponse("getAccountInfo: missing owner".to_string())
            })?
            .to_string();
        let parsed_program_label = account_value
            .pointer("/data/program")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let parsed_json = account_value
            .pointer("/data/parsed")
            .cloned()
            .ok_or_else(|| {
                CoreError::MalformedResponse(
                    "getAccountInfo: account data was not jsonParsed (unknown program?)"
                        .to_string(),
                )
            })?;
        Ok(Some(ParsedAccountInfo {
            owner_program,
            parsed_program_label,
            parsed_json,
        }))
    }

    /// `getAccountInfo` with base64 encoding, returning the raw account data
    /// bytes. `Ok(None)` means the account does not exist.
    pub fn get_account_data(&self, address: &Pubkey) -> Result<Option<Vec<u8>>, CoreError> {
        let result = self.call(
            "getAccountInfo",
            json!([address.to_string(), { "encoding": "base64" }]),
        )?;
        let account_value = match result.get("value") {
            Some(Value::Null) | None => return Ok(None),
            Some(account_value) => account_value,
        };
        let base64_text = account_value
            .pointer("/data/0")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                CoreError::MalformedResponse("getAccountInfo: missing base64 data".to_string())
            })?;
        base64::engine::general_purpose::STANDARD
            .decode(base64_text)
            .map(Some)
            .map_err(|decode_error| {
                CoreError::MalformedResponse(format!(
                    "getAccountInfo: undecodable base64 data: {decode_error}"
                ))
            })
    }

    /// Cheap existence probe: `getAccountInfo` with a zero-length dataSlice,
    /// so the node returns no account data bytes at all.
    pub fn account_exists(&self, address: &Pubkey) -> Result<bool, CoreError> {
        let result = self.call(
            "getAccountInfo",
            json!([
                address.to_string(),
                { "encoding": "base64", "dataSlice": { "offset": 0, "length": 0 } }
            ]),
        )?;
        Ok(!matches!(result.get("value"), Some(Value::Null) | None))
    }

    /// `getTokenLargestAccounts`: the 20 largest token accounts of a mint.
    pub fn get_token_largest_accounts(
        &self,
        mint: &Pubkey,
    ) -> Result<Vec<LargestTokenAccount>, CoreError> {
        let result = self.call("getTokenLargestAccounts", json!([mint.to_string()]))?;
        let entries = result
            .pointer("/value")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                CoreError::MalformedResponse(
                    "getTokenLargestAccounts: missing value array".to_string(),
                )
            })?;
        let mut largest_accounts = Vec::with_capacity(entries.len());
        for entry in entries {
            let address = entry
                .get("address")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    CoreError::MalformedResponse(
                        "getTokenLargestAccounts: entry missing address".to_string(),
                    )
                })?
                .to_string();
            let amount_base_units = entry
                .get("amount")
                .and_then(Value::as_str)
                .and_then(|amount_text| amount_text.parse::<u128>().ok())
                .ok_or_else(|| {
                    CoreError::MalformedResponse(
                        "getTokenLargestAccounts: entry missing integer amount".to_string(),
                    )
                })?;
            largest_accounts.push(LargestTokenAccount {
                address,
                amount_base_units,
            });
        }
        Ok(largest_accounts)
    }

    /// `getSignaturesForAddress`, newest first. `until` excludes that
    /// signature and everything older, which is the cursor pattern a
    /// stateless watcher needs (sourceRef:
    /// https://solana.com/docs/rpc/http/getsignaturesforaddress).
    pub fn get_signatures_for_address(
        &self,
        address: &Pubkey,
        until_signature: Option<&str>,
        limit: u16,
    ) -> Result<Vec<SignatureRecord>, CoreError> {
        let mut options = json!({ "limit": limit, "commitment": "confirmed" });
        if let Some(cursor_signature) = until_signature {
            options["until"] = json!(cursor_signature);
        }
        let result = self.call(
            "getSignaturesForAddress",
            json!([address.to_string(), options]),
        )?;
        let entries = result.as_array().ok_or_else(|| {
            CoreError::MalformedResponse(
                "getSignaturesForAddress: result is not an array".to_string(),
            )
        })?;
        let mut signature_records = Vec::with_capacity(entries.len());
        for entry in entries {
            let signature = entry
                .get("signature")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    CoreError::MalformedResponse(
                        "getSignaturesForAddress: entry missing signature".to_string(),
                    )
                })?
                .to_string();
            let slot = entry.get("slot").and_then(Value::as_u64).ok_or_else(|| {
                CoreError::MalformedResponse(
                    "getSignaturesForAddress: entry missing slot".to_string(),
                )
            })?;
            signature_records.push(SignatureRecord {
                signature,
                slot,
                failed: !matches!(entry.get("err"), Some(Value::Null) | None),
            });
        }
        Ok(signature_records)
    }

    /// `getTransaction` with jsonParsed encoding and v0 support. `Ok(None)`
    /// means the node does not have the transaction. The raw JSON is
    /// returned; `payment_verify` computes typed facts from it.
    pub fn get_transaction_json(&self, signature: &str) -> Result<Option<Value>, CoreError> {
        let result = self.call(
            "getTransaction",
            json!([
                signature,
                { "encoding": "jsonParsed", "maxSupportedTransactionVersion": 0,
                  "commitment": "confirmed" }
            ]),
        )?;
        if result.is_null() {
            return Ok(None);
        }
        Ok(Some(result))
    }
}
