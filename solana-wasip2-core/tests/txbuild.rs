//! Host tests for unsigned v0 transaction construction. Every assertion
//! decodes the produced base64 back into a `VersionedTransaction` and checks
//! the wire-level facts a wallet or signer would see.
//!
//! Instruction discriminants asserted here were read from the program
//! interface sources: `TransferChecked` packs tag byte 12 then amount u64 LE
//! then decimals u8 (sourceRef: spl-token-interface-2.0.0/src/instruction.rs,
//! pack), and `AdvanceNonceAccount` is SystemInstruction variant 4, bincode
//! u32 LE (sourceRef: solana-system-interface-3.2.0/src/instruction.rs).

use base64::Engine as _;
use solana_hash::Hash;
use solana_pubkey::Pubkey;
use solana_transaction::versioned::VersionedTransaction;
use solana_wasip2_core::addresses::{memo_program_id, token_program_id};
use solana_wasip2_core::txbuild::{
    build_spl_transfer_transaction, build_unsigned_v0_transaction, TransactionLifetime,
    TransferCheckedSpec,
};

/// Wallet, mint, and its on-chain ATA, verified as a live triple on mainnet
/// on 2026-07-21: the ATA below exists, is owned by the wallet, and holds
/// the mint (sourceRef: getAccountInfo on
/// FGETo8T8wMcN2wCjav8VK6eh3dLk63evNDPxzLSJra8B).
const SENDER_WALLET: &str = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const SENDER_USDC_ATA: &str = "FGETo8T8wMcN2wCjav8VK6eh3dLk63evNDPxzLSJra8B";
const RECIPIENT_WALLET: &str = "2apBGMsS6ti9RyF5TwQTDswXBWskiJP2LD4cUEDqYJjk";

fn example_blockhash() -> Hash {
    "D277KYCrJsSujJyqKpwwaGW2v8QRFtYnJ3qAC39SZ1tF"
        .parse()
        .unwrap()
}

fn usdc_transfer_spec() -> TransferCheckedSpec {
    TransferCheckedSpec {
        token_program_id: token_program_id(),
        mint: USDC_MINT.parse().unwrap(),
        sender_wallet: SENDER_WALLET.parse().unwrap(),
        recipient_wallet: RECIPIENT_WALLET.parse().unwrap(),
        amount_base_units: 25_000_000,
        decimals: 6,
        create_recipient_ata: true,
        memo_text: Some("order#412".to_string()),
    }
}

fn decode_transaction(unsigned_transaction_base64: &str) -> VersionedTransaction {
    let transaction_bytes = base64::engine::general_purpose::STANDARD
        .decode(unsigned_transaction_base64)
        .expect("output must be valid base64");
    bincode::deserialize(&transaction_bytes).expect("output must deserialize as a transaction")
}

#[test]
fn a_recent_blockhash_transfer_has_the_expected_wire_shape() {
    let built = build_spl_transfer_transaction(
        &usdc_transfer_spec(),
        &TransactionLifetime::RecentBlockhash(example_blockhash()),
    )
    .unwrap();

    assert_eq!(built.source_token_account.to_string(), SENDER_USDC_ATA);

    let transaction = decode_transaction(&built.transaction.unsigned_transaction_base64);
    // Exactly one signer (the sender), and its slot is a zeroed placeholder.
    assert_eq!(transaction.signatures.len(), 1);
    assert_eq!(transaction.signatures[0].as_ref(), &[0u8; 64]);
    assert_eq!(transaction.message.header().num_required_signatures, 1);
    assert_eq!(
        transaction.message.static_account_keys()[0].to_string(),
        SENDER_WALLET
    );
    assert_eq!(*transaction.message.recent_blockhash(), example_blockhash());

    // Instruction order: create-ATA, memo, transfer.
    let compiled_instructions = transaction.message.instructions();
    assert_eq!(compiled_instructions.len(), 3);
    let static_keys = transaction.message.static_account_keys();
    let program_id_of = |instruction_index: usize| {
        static_keys[compiled_instructions[instruction_index].program_id_index as usize]
    };
    assert_eq!(
        program_id_of(0),
        spl_associated_token_account_interface::program::id()
    );
    assert_eq!(program_id_of(1), memo_program_id());
    assert_eq!(program_id_of(2), token_program_id());

    // TransferChecked payload: tag 12, amount 25_000_000 LE, decimals 6.
    let transfer_data = &compiled_instructions[2].data;
    assert_eq!(transfer_data[0], 12);
    assert_eq!(
        u64::from_le_bytes(transfer_data[1..9].try_into().unwrap()),
        25_000_000
    );
    assert_eq!(transfer_data[9], 6);

    // The memo body is the configured text.
    assert_eq!(compiled_instructions[1].data, b"order#412");

    assert_eq!(built.transaction.required_signers.len(), 1);
    assert_eq!(built.transaction.instruction_count, 3);
}

#[test]
fn a_durable_nonce_transfer_advances_the_nonce_first() {
    let nonce_account: Pubkey = "4nEWKw6W8uXmF5u9qyDTziARAZdC4YNxFnhgpzsJVDBE"
        .parse()
        .unwrap();
    let sender: Pubkey = SENDER_WALLET.parse().unwrap();
    let nonce_value = example_blockhash();
    let built = build_spl_transfer_transaction(
        &usdc_transfer_spec(),
        &TransactionLifetime::DurableNonce {
            nonce_account,
            nonce_authority: sender,
            nonce_value,
        },
    )
    .unwrap();

    let transaction = decode_transaction(&built.transaction.unsigned_transaction_base64);
    // The nonce value rides in the recent_blockhash field.
    assert_eq!(*transaction.message.recent_blockhash(), nonce_value);

    let compiled_instructions = transaction.message.instructions();
    assert_eq!(compiled_instructions.len(), 4);
    let static_keys = transaction.message.static_account_keys();

    // First instruction: system program AdvanceNonceAccount (variant 4,
    // bincode u32 LE), with the nonce account as its first account.
    let first_instruction = &compiled_instructions[0];
    assert_eq!(
        static_keys[first_instruction.program_id_index as usize],
        solana_sdk_ids::system_program::id()
    );
    assert_eq!(first_instruction.data, vec![4, 0, 0, 0]);
    assert_eq!(
        static_keys[first_instruction.accounts[0] as usize],
        nonce_account
    );

    // Same single signer: the sender is also the nonce authority.
    assert_eq!(transaction.message.header().num_required_signatures, 1);
}

#[test]
fn a_distinct_nonce_authority_becomes_a_second_required_signer() {
    let nonce_account: Pubkey = "4nEWKw6W8uXmF5u9qyDTziARAZdC4YNxFnhgpzsJVDBE"
        .parse()
        .unwrap();
    let distinct_authority: Pubkey = RECIPIENT_WALLET.parse().unwrap();
    let built = build_spl_transfer_transaction(
        &usdc_transfer_spec(),
        &TransactionLifetime::DurableNonce {
            nonce_account,
            nonce_authority: distinct_authority,
            nonce_value: example_blockhash(),
        },
    )
    .unwrap();
    assert_eq!(built.transaction.required_signers.len(), 2);
    assert!(built
        .transaction
        .required_signers
        .contains(&distinct_authority));
}

#[test]
fn skipping_ata_creation_and_memo_yields_a_single_instruction() {
    let mut spec = usdc_transfer_spec();
    spec.create_recipient_ata = false;
    spec.memo_text = None;
    let built = build_spl_transfer_transaction(
        &spec,
        &TransactionLifetime::RecentBlockhash(example_blockhash()),
    )
    .unwrap();
    let transaction = decode_transaction(&built.transaction.unsigned_transaction_base64);
    assert_eq!(transaction.message.instructions().len(), 1);
}

#[test]
fn the_hand_rolled_transfer_checked_matches_the_interface_crates_builder() {
    // The core hand-rolls TransferChecked so it can target Token-2022 (the
    // classic interface builder rejects that program id). This pins the
    // hand-rolled bytes and account metas to the interface crate's output,
    // so the encoding can never drift from the source of truth.
    let source: Pubkey = SENDER_USDC_ATA.parse().unwrap();
    let mint: Pubkey = USDC_MINT.parse().unwrap();
    let destination: Pubkey = "4nEWKw6W8uXmF5u9qyDTziARAZdC4YNxFnhgpzsJVDBE"
        .parse()
        .unwrap();
    let owner: Pubkey = SENDER_WALLET.parse().unwrap();

    let hand_rolled = solana_wasip2_core::txbuild::build_transfer_checked_instruction(
        &token_program_id(),
        &source,
        &mint,
        &destination,
        &owner,
        25_000_000,
        6,
    );
    let interface_built = spl_token_interface::instruction::transfer_checked(
        &token_program_id(),
        &source,
        &mint,
        &destination,
        &owner,
        &[],
        25_000_000,
        6,
    )
    .unwrap();
    assert_eq!(hand_rolled.program_id, interface_built.program_id);
    assert_eq!(hand_rolled.data, interface_built.data);
    assert_eq!(hand_rolled.accounts.len(), interface_built.accounts.len());
    for (hand_rolled_meta, interface_meta) in hand_rolled
        .accounts
        .iter()
        .zip(interface_built.accounts.iter())
    {
        assert_eq!(hand_rolled_meta.pubkey, interface_meta.pubkey);
        assert_eq!(hand_rolled_meta.is_signer, interface_meta.is_signer);
        assert_eq!(hand_rolled_meta.is_writable, interface_meta.is_writable);
    }
}

#[test]
fn building_with_no_instructions_still_compiles_a_valid_message() {
    let sender: Pubkey = SENDER_WALLET.parse().unwrap();
    let built = build_unsigned_v0_transaction(&sender, &[], example_blockhash()).unwrap();
    let transaction = decode_transaction(&built.unsigned_transaction_base64);
    assert_eq!(transaction.message.static_account_keys()[0], sender);
    assert_eq!(built.instruction_count, 0);
}
