// Cross-stack transaction oracle.
//
// Rebuilds the vectors from ../../examples/emit_oracle_vectors.rs with the
// reference JavaScript stack (@solana/web3.js + @solana/spl-token), then
// compares the two unsigned transactions SEMANTICALLY: fee payer, blockhash,
// required signers, and every instruction's program id, ordered account
// list with signer/writable flags, and exact data bytes.
//
// Why not byte equality: the two reference stacks themselves order
// non-signer static account keys differently (Rust solana-message
// CompiledKeys vs web3.js compileToV0Message tie-breaking), so byte-equal
// output across stacks does not exist. Our bytes come from the canonical
// Rust compiler; this oracle proves the JavaScript stack reads them as the
// same transaction it would have built itself.
//
// Run:
//   cargo run --example emit_oracle_vectors > tools/byte-oracle/vectors.json
//   cd tools/byte-oracle && npm install && node check.mjs
//
// Exits 0 only when every vector is semantically identical.

import { readFileSync } from "node:fs";
import {
  PublicKey,
  SystemProgram,
  TransactionInstruction,
  TransactionMessage,
  VersionedTransaction,
} from "@solana/web3.js";
import {
  createAssociatedTokenAccountIdempotentInstruction,
  createTransferCheckedInstruction,
  getAssociatedTokenAddressSync,
} from "@solana/spl-token";

// SPL Memo v3 program id (sourceRef: solana-program/memo interface declare_id).
const MEMO_PROGRAM_ID = new PublicKey("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");

const { inputs, vectors } = JSON.parse(readFileSync(new URL("./vectors.json", import.meta.url)));

const sender = new PublicKey(inputs.sender);
const recipient = new PublicKey(inputs.recipient);
const mint = new PublicKey(inputs.mint);
const nonceAccount = new PublicKey(inputs.nonce_account);
const senderAta = getAssociatedTokenAddressSync(mint, sender);
const recipientAta = getAssociatedTokenAddressSync(mint, recipient);

const transferInstruction = () =>
  createTransferCheckedInstruction(
    senderAta, mint, recipientAta, sender,
    BigInt(inputs.amount_base_units), inputs.decimals,
  );
const createAtaInstruction = () =>
  createAssociatedTokenAccountIdempotentInstruction(sender, recipientAta, recipient, mint);
const memoInstruction = () =>
  new TransactionInstruction({
    programId: MEMO_PROGRAM_ID,
    keys: [{ pubkey: sender, isSigner: true, isWritable: false }],
    data: Buffer.from(inputs.memo, "utf8"),
  });
const nonceAdvanceInstruction = () =>
  SystemProgram.nonceAdvance({ noncePubkey: nonceAccount, authorizedPubkey: sender });

function buildReferenceTransaction(instructions) {
  const message = new TransactionMessage({
    payerKey: sender,
    recentBlockhash: inputs.blockhash,
    instructions,
  }).compileToV0Message();
  return new VersionedTransaction(message);
}

/// Project a VersionedTransaction into a stack-independent shape.
function semanticShape(transaction) {
  const message = transaction.message;
  const keys = message.staticAccountKeys.map((accountKey) => accountKey.toBase58());
  return {
    payer: keys[0],
    blockhash: message.recentBlockhash,
    requiredSigners: keys.slice(0, message.header.numRequiredSignatures).sort(),
    zeroedSignatures: transaction.signatures.every((signature) =>
      signature.every((byte) => byte === 0),
    ),
    instructions: message.compiledInstructions.map((compiled) => ({
      programId: keys[compiled.programIdIndex],
      dataHex: Buffer.from(compiled.data).toString("hex"),
      accounts: compiled.accountKeyIndexes.map((accountIndex) => ({
        pubkey: keys[accountIndex],
        signer: message.isAccountSigner(accountIndex),
        writable: message.isAccountWritable(accountIndex),
      })),
    })),
  };
}

const referenceBuilds = {
  blockhash_full: buildReferenceTransaction([
    createAtaInstruction(), memoInstruction(), transferInstruction(),
  ]),
  blockhash_bare: buildReferenceTransaction([transferInstruction()]),
  durable_nonce_full: buildReferenceTransaction([
    nonceAdvanceInstruction(), createAtaInstruction(), memoInstruction(), transferInstruction(),
  ]),
};

let failures = 0;
for (const [vectorName, rustBase64] of Object.entries(vectors)) {
  const rustTransaction = VersionedTransaction.deserialize(Buffer.from(rustBase64, "base64"));
  const rustShape = semanticShape(rustTransaction);
  const referenceShape = semanticShape(referenceBuilds[vectorName]);
  const bytesEqual =
    Buffer.from(referenceBuilds[vectorName].serialize()).toString("base64") === rustBase64;
  if (JSON.stringify(rustShape) === JSON.stringify(referenceShape)) {
    console.log(
      `PASS ${vectorName}: semantically identical to web3.js ` +
        `(${rustShape.instructions.length} instructions; ` +
        `byte layout ${bytesEqual ? "identical" : "differs only in account-table order"})`,
    );
  } else {
    failures += 1;
    console.log(`FAIL ${vectorName}`);
    console.log(`  rust:    ${JSON.stringify(rustShape)}`);
    console.log(`  web3.js: ${JSON.stringify(referenceShape)}`);
  }
}
process.exit(failures === 0 ? 0 : 1);
