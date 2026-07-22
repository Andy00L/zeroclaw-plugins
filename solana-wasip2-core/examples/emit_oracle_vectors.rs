//! Emit deterministic unsigned-transaction vectors for the byte oracle.
//!
//! The oracle (tools/byte-oracle/check.mjs) rebuilds the same transactions
//! with the reference JavaScript stack (@solana/web3.js + @solana/spl-token)
//! and asserts byte equality with the base64 printed here. Run:
//!
//!   cargo run --example emit_oracle_vectors > tools/byte-oracle/vectors.json

use solana_wasip2_core::addresses::{token_program_id, Pubkey};
use solana_wasip2_core::txbuild::{
    build_spl_transfer_transaction, TransactionLifetime, TransferCheckedSpec,
};

/// Fixed inputs, chosen from the repository's verified test constants so
/// the oracle needs no network: sender and its real USDC ATA, a recipient,
/// a fixed blockhash, and a fixed nonce account.
const SENDER_WALLET: &str = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
const RECIPIENT_WALLET: &str = "2apBGMsS6ti9RyF5TwQTDswXBWskiJP2LD4cUEDqYJjk";
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const NONCE_ACCOUNT: &str = "4nEWKw6W8uXmF5u9qyDTziARAZdC4YNxFnhgpzsJVDBE";
const FIXED_BLOCKHASH: &str = "D277KYCrJsSujJyqKpwwaGW2v8QRFtYnJ3qAC39SZ1tF";

fn base_spec() -> TransferCheckedSpec {
    TransferCheckedSpec {
        token_program_id: token_program_id(),
        mint: USDC_MINT.parse().unwrap(),
        sender_wallet: SENDER_WALLET.parse().unwrap(),
        recipient_wallet: RECIPIENT_WALLET.parse().unwrap(),
        amount_base_units: 25_000_000,
        decimals: 6,
        create_recipient_ata: true,
        memo_text: Some("oracle#1".to_string()),
    }
}

fn main() {
    let blockhash_lifetime = TransactionLifetime::RecentBlockhash(FIXED_BLOCKHASH.parse().unwrap());
    let nonce_lifetime = TransactionLifetime::DurableNonce {
        nonce_account: NONCE_ACCOUNT.parse().unwrap(),
        nonce_authority: SENDER_WALLET.parse::<Pubkey>().unwrap(),
        nonce_value: FIXED_BLOCKHASH.parse().unwrap(),
    };

    let mut bare_spec = base_spec();
    bare_spec.create_recipient_ata = false;
    bare_spec.memo_text = None;

    let vectors = serde_json::json!({
        "inputs": {
            "sender": SENDER_WALLET,
            "recipient": RECIPIENT_WALLET,
            "mint": USDC_MINT,
            "nonce_account": NONCE_ACCOUNT,
            "blockhash": FIXED_BLOCKHASH,
            "amount_base_units": 25_000_000u64,
            "decimals": 6,
            "memo": "oracle#1"
        },
        "vectors": {
            "blockhash_full": build_spl_transfer_transaction(&base_spec(), &blockhash_lifetime)
                .unwrap()
                .transaction
                .unsigned_transaction_base64,
            "blockhash_bare": build_spl_transfer_transaction(&bare_spec, &blockhash_lifetime)
                .unwrap()
                .transaction
                .unsigned_transaction_base64,
            "durable_nonce_full": build_spl_transfer_transaction(&base_spec(), &nonce_lifetime)
                .unwrap()
                .transaction
                .unsigned_transaction_base64,
        }
    });
    println!("{}", serde_json::to_string_pretty(&vectors).expect("vectors serialize"));
}
