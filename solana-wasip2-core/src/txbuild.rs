//! Unsigned versioned (v0) transaction construction.
//!
//! Every builder returns base64 of a fully serialized transaction whose
//! signature slots are zero-filled placeholders. That is the wire format
//! wallets, `simulateTransaction` (with sigVerify false), and Solana Pay
//! transaction responses expect for an unsigned transaction. This crate never
//! touches a private key: signing is the human's or the host's job.

use base64::Engine as _;
use solana_hash::Hash;
use solana_instruction::{AccountMeta, Instruction};
use solana_message::{v0, VersionedMessage};
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;

use crate::addresses::derive_associated_token_address;
use crate::error::CoreError;

/// How the transaction proves its recency.
#[derive(Debug)]
pub enum TransactionLifetime {
    /// A recent blockhash: valid for about 150 slots (~90 seconds).
    RecentBlockhash(Hash),
    /// A durable nonce: `AdvanceNonceAccount` is prepended as the first
    /// instruction and the stored nonce hash becomes the recent_blockhash,
    /// so the transaction stays signable until the nonce advances
    /// (sourceRef: https://solana.com/docs/core/transactions/durable-nonces).
    DurableNonce {
        nonce_account: Pubkey,
        nonce_authority: Pubkey,
        nonce_value: Hash,
    },
}

/// What to transfer and how, for an SPL `TransferChecked` transaction.
#[derive(Debug)]
pub struct TransferCheckedSpec {
    /// Classic SPL Token or Token-2022 program id; `TransferChecked` has the
    /// same encoding under both.
    pub token_program_id: Pubkey,
    pub mint: Pubkey,
    /// The wallet that owns the source tokens, pays fees, and signs.
    pub sender_wallet: Pubkey,
    /// The recipient's wallet address (not their token account).
    pub recipient_wallet: Pubkey,
    pub amount_base_units: u64,
    pub decimals: u8,
    /// Prepend an idempotent create-ATA instruction for the recipient.
    pub create_recipient_ata: bool,
    /// Optional SPL Memo text, placed immediately before the transfer
    /// instruction (the Solana Pay convention; sourceRef:
    /// https://docs.solanapay.com/spec, memo field).
    pub memo_text: Option<String>,
}

/// A built unsigned transaction plus the facts a caller needs to describe it.
#[derive(Debug)]
pub struct BuiltTransaction {
    pub unsigned_transaction_base64: String,
    /// Static account keys that must sign, in header order.
    pub required_signers: Vec<Pubkey>,
    pub instruction_count: usize,
}

/// A built unsigned SPL transfer with the derived token accounts.
#[derive(Debug)]
pub struct BuiltTransfer {
    pub transaction: BuiltTransaction,
    pub source_token_account: Pubkey,
    pub destination_token_account: Pubkey,
}

/// Build a `TransferChecked` instruction for either token program. The
/// classic interface crate's builder rejects the Token-2022 program id, and
/// the Token-2022 interface crate drags the confidential-transfer proof
/// stack, so the encoding is produced here: tag byte 12, amount u64 LE,
/// decimals u8, over accounts source (writable), mint, destination
/// (writable), owner (signer). A host test pins this byte-for-byte against
/// spl-token-interface's own builder (sourceRef:
/// spl-token-interface-2.0.0/src/instruction.rs, transfer_checked and
/// TokenInstruction::pack).
#[allow(clippy::too_many_arguments)]
pub fn build_transfer_checked_instruction(
    token_program_id: &Pubkey,
    source_token_account: &Pubkey,
    mint: &Pubkey,
    destination_token_account: &Pubkey,
    owner_wallet: &Pubkey,
    amount_base_units: u64,
    decimals: u8,
) -> Instruction {
    // Tag 12 = TransferChecked, then amount, then decimals.
    let mut instruction_data = Vec::with_capacity(10);
    instruction_data.push(12);
    instruction_data.extend_from_slice(&amount_base_units.to_le_bytes());
    instruction_data.push(decimals);
    Instruction {
        program_id: *token_program_id,
        accounts: vec![
            AccountMeta::new(*source_token_account, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new(*destination_token_account, false),
            AccountMeta::new_readonly(*owner_wallet, true),
        ],
        data: instruction_data,
    }
}

/// Compile instructions into a v0 message, wrap it in a `VersionedTransaction`
/// with zero-filled signature placeholders, and base64-encode the canonical
/// bincode serialization.
pub fn build_unsigned_v0_transaction(
    fee_payer: &Pubkey,
    instructions: &[Instruction],
    recent_blockhash: Hash,
) -> Result<BuiltTransaction, CoreError> {
    let message_v0 = v0::Message::try_compile(fee_payer, instructions, &[], recent_blockhash)
        .map_err(|compile_error| CoreError::TxCompileFailed(compile_error.to_string()))?;
    let versioned_message = VersionedMessage::V0(message_v0);

    let required_signer_count = versioned_message.header().num_required_signatures as usize;
    let required_signers =
        versioned_message.static_account_keys()[..required_signer_count].to_vec();
    let placeholder_signatures = vec![Signature::default(); required_signer_count];

    let versioned_transaction = VersionedTransaction {
        signatures: placeholder_signatures,
        message: versioned_message,
    };
    let transaction_bytes = bincode::serialize(&versioned_transaction).map_err(|encode_error| {
        CoreError::TxCompileFailed(format!("serialization failed: {encode_error}"))
    })?;

    Ok(BuiltTransaction {
        unsigned_transaction_base64: base64::engine::general_purpose::STANDARD
            .encode(transaction_bytes),
        required_signers,
        instruction_count: instructions.len(),
    })
}

/// Build an unsigned SPL `TransferChecked` transaction between two wallets'
/// associated token accounts. Instruction order: advance-nonce (durable nonce
/// lifetime only, must be first), idempotent create-ATA (optional), memo
/// (optional, immediately before the transfer), transfer-checked.
pub fn build_spl_transfer_transaction(
    spec: &TransferCheckedSpec,
    lifetime: &TransactionLifetime,
) -> Result<BuiltTransfer, CoreError> {
    let source_token_account =
        derive_associated_token_address(&spec.sender_wallet, &spec.mint, &spec.token_program_id);
    let destination_token_account =
        derive_associated_token_address(&spec.recipient_wallet, &spec.mint, &spec.token_program_id);

    let mut instructions: Vec<Instruction> = Vec::with_capacity(4);
    let recent_blockhash = match lifetime {
        TransactionLifetime::RecentBlockhash(blockhash) => *blockhash,
        TransactionLifetime::DurableNonce {
            nonce_account,
            nonce_authority,
            nonce_value,
        } => {
            instructions.push(solana_system_interface::instruction::advance_nonce_account(
                nonce_account,
                nonce_authority,
            ));
            *nonce_value
        }
    };

    if spec.create_recipient_ata {
        instructions.push(
            spl_associated_token_account_interface::instruction::create_associated_token_account_idempotent(
                &spec.sender_wallet,
                &spec.recipient_wallet,
                &spec.mint,
                &spec.token_program_id,
            ),
        );
    }
    if let Some(memo_text) = &spec.memo_text {
        // The sender signs the transaction, so it is the memo's signer too.
        instructions.push(spl_memo_interface::instruction::build_memo(
            &crate::addresses::memo_program_id(),
            memo_text.as_bytes(),
            &[&spec.sender_wallet],
        ));
    }
    instructions.push(build_transfer_checked_instruction(
        &spec.token_program_id,
        &source_token_account,
        &spec.mint,
        &destination_token_account,
        &spec.sender_wallet,
        spec.amount_base_units,
        spec.decimals,
    ));

    let transaction =
        build_unsigned_v0_transaction(&spec.sender_wallet, &instructions, recent_blockhash)?;
    Ok(BuiltTransfer {
        transaction,
        source_token_account,
        destination_token_account,
    })
}
