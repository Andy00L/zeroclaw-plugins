//! Host tests for durable nonce account parsing. State bytes are produced
//! with the same solana-nonce types and bincode encoding the System program
//! uses on chain, so the round trip is faithful.

use solana_hash::Hash;
use solana_nonce::state::{Data, DurableNonce, State};
use solana_nonce::versions::Versions;
use solana_pubkey::Pubkey;
use solana_wasip2_core::error::CoreError;
use solana_wasip2_core::nonce::parse_nonce_account_data;

fn example_authority() -> Pubkey {
    "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM".parse().unwrap()
}

fn example_blockhash() -> Hash {
    "D277KYCrJsSujJyqKpwwaGW2v8QRFtYnJ3qAC39SZ1tF".parse().unwrap()
}

#[test]
fn parses_an_initialized_current_nonce_account() {
    let durable_nonce = DurableNonce::from_blockhash(&example_blockhash());
    let nonce_state = Versions::new(State::Initialized(Data::new(
        example_authority(),
        durable_nonce,
        5_000,
    )));
    let account_data = bincode::serialize(&nonce_state).unwrap();

    let nonce_info = parse_nonce_account_data(&account_data).unwrap();
    assert_eq!(nonce_info.authority, example_authority());
    // The durable nonce is domain-separated from the blockhash it derives
    // from; the parsed value must equal the derived hash, not the blockhash.
    assert_eq!(nonce_info.nonce_value, *durable_nonce.as_hash());
    assert_ne!(nonce_info.nonce_value, example_blockhash());
}

#[test]
fn rejects_a_legacy_nonce_account() {
    let durable_nonce = DurableNonce::from_blockhash(&example_blockhash());
    let legacy_state = Versions::Legacy(Box::new(State::Initialized(Data::new(
        example_authority(),
        durable_nonce,
        5_000,
    ))));
    let account_data = bincode::serialize(&legacy_state).unwrap();
    match parse_nonce_account_data(&account_data) {
        Err(CoreError::InvalidNonceAccount(message)) => {
            assert!(message.contains("legacy"), "got: {message}");
        }
        other => panic!("expected InvalidNonceAccount, got {other:?}"),
    }
}

#[test]
fn rejects_an_uninitialized_nonce_account() {
    let uninitialized_state = Versions::new(State::Uninitialized);
    let account_data = bincode::serialize(&uninitialized_state).unwrap();
    match parse_nonce_account_data(&account_data) {
        Err(CoreError::InvalidNonceAccount(message)) => {
            assert!(message.contains("uninitialized"), "got: {message}");
        }
        other => panic!("expected InvalidNonceAccount, got {other:?}"),
    }
}

#[test]
fn rejects_garbage_and_truncated_data() {
    assert!(matches!(
        parse_nonce_account_data(&[0xFF; 80]),
        Err(CoreError::InvalidNonceAccount(_))
    ));
    let durable_nonce = DurableNonce::from_blockhash(&example_blockhash());
    let valid_state = Versions::new(State::Initialized(Data::new(
        example_authority(),
        durable_nonce,
        5_000,
    )));
    let mut account_data = bincode::serialize(&valid_state).unwrap();
    account_data.truncate(account_data.len() / 2);
    assert!(matches!(
        parse_nonce_account_data(&account_data),
        Err(CoreError::InvalidNonceAccount(_))
    ));
    assert!(matches!(
        parse_nonce_account_data(&[]),
        Err(CoreError::InvalidNonceAccount(_))
    ));
}
