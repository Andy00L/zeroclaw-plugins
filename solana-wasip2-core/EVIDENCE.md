# Evidence

Every claim the suite's READMEs make, with the command that reproduces it.
Regenerated 2026-07-21. One command runs everything:

```bash
cd solana-wasip2-core && ./prove.sh
```

Exits 0 only when all checks pass. Requires Rust 1.96+ with the
`wasm32-wasip2` target and node (for the oracle).

## Test matrix (host, zero network)

| Crate | Host tests | Adversarial subset |
|---|---|---|
| solana-wasip2-core | 56 | URL parameter injection, hostile-name sanitization, legacy-nonce rejection, malformed RPC shapes |
| token-risk-check | 10 | hostile token-name verdict injection, typoed config, non-mint accounts |
| solana-pay-request | 11 | smuggled recipient argument, URL metacharacter injection, broken token configs |
| spl-transfer-build | 13 | unlisted recipient, over-cap amount, smuggled sender, decimals mismatch, risk-gate bypass attempt, foreign nonce authority |
| payment-watch | 12 | verification redirection, reference-touch spoof, malformed cursor, failed-transaction spam |
| **Total** | **102** | 38 tests assert refusal or containment by name |

Injection paths additionally assert that zero RPC calls left the sandbox
before the refusal (`rpc_call_count == 0` / `requested_urls.is_empty()`).

## Cross-stack transaction oracle

`cargo run --example emit_oracle_vectors` emits three unsigned
transactions built by this crate (plain transfer; create-ATA + memo +
transfer; durable-nonce + create-ATA + memo + transfer).
`tools/byte-oracle/check.mjs` rebuilds them with `@solana/web3.js` +
`@solana/spl-token` and compares fee payer, blockhash, required signers,
zeroed signature slots, and every instruction's program id, ordered
account metas with signer/writable flags, and exact data bytes. Result:

```
PASS blockhash_bare: semantically identical to web3.js (1 instructions; byte layout differs only in account-table order)
PASS blockhash_full: semantically identical to web3.js (3 instructions; byte layout differs only in account-table order)
PASS durable_nonce_full: semantically identical to web3.js (4 instructions; byte layout differs only in account-table order)
```

Finding worth stating: byte-identical output across the two reference
stacks does not exist, because Rust's `solana-message` compiler and
web3.js `compileToV0Message` order non-signer static account keys
differently. This suite's bytes come from the canonical Rust compiler;
the oracle proves the JavaScript stack reads them as exactly the
transaction it would have built itself.

## Wire-level pins against primary sources

- `TransferChecked` encoding pinned byte-for-byte against
  `spl-token-interface`'s own builder (core test
  `the_hand_rolled_transfer_checked_matches_the_interface_crates_builder`).
- ATA derivation triple-verified: interface crate == manual seed
  derivation == a live mainnet account
  (`FGETo8T8wMcN2wCjav8VK6eh3dLk63evNDPxzLSJra8B`, checked on chain
  2026-07-21).
- `AdvanceNonceAccount` asserted as the first instruction with the nonce
  value in the recent_blockhash field, decoded from the produced base64.
- Program ids read from each program's `declare_id!` source line, not from
  memory (sourceRefs in `addresses.rs`).

## Real-data fixtures

`tests/fixtures/` are mainnet captures from 2026-07-21, not synthetic:
USDC and PYUSD mints (PYUSD exercises permanent delegate, transfer hook,
transfer fee, confidential transfer extensions), a genuinely failed
transaction in the signature list, a real transfer with a fee-splitter
(recipient received 1,999,740 of the 2,000,000 sent), and a live HTTP 429
from the public RPC. The one synthesized response
(`getTokenLargestAccounts`, which every public endpoint rate-limited
during capture) is labeled as such where used.

## Output size discipline

The bounty warns judges will call execute and count tokens. Worst-case
report sizes are asserted in tests: token-risk-check under 1,600 chars for
the extension-heavy PYUSD case, payment-watch under 1,000 chars for a
settled invoice with all notes present.

## In progress (not yet evidence)

Planned artifacts tracked in PUSH_FURTHER.md, listed here so nothing reads
as more than it is: the upstream CI gate run (blocked on publishing
`solana-wasip2-core` to crates.io; the gate's snapshot cannot see the path
dependency, verified empirically with `test_rc=125`), a live devnet
durable-nonce delayed-signing run, and the in-host Telegram approval-gate
transcript.
