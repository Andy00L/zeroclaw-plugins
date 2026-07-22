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

## Upstream gate, run exactly as upstream CI runs it

The repository's own `tools/ci/validate_components.sh` (isolated snapshot
per plugin, `--locked`, clippy `-D warnings` on both targets, release
component build) on 2026-07-22, with `solana-wasip2-core 0.1.0` resolved
from crates.io (https://crates.io/crates/solana-wasip2-core):

```
token-risk-check:   test_rc=0 tests_passed=10 clippy_rc=0 wasm_clippy_rc=0 build_rc=0 artifact_bytes=368979
solana-pay-request: test_rc=0 tests_passed=11 clippy_rc=0 wasm_clippy_rc=0 build_rc=0 artifact_bytes=221874
spl-transfer-build: test_rc=0 tests_passed=13 clippy_rc=0 wasm_clippy_rc=0 build_rc=0 artifact_bytes=490408
payment-watch:      test_rc=0 tests_passed=12 clippy_rc=0 wasm_clippy_rc=0 build_rc=0 artifact_bytes=390571
```

Context that makes these rows non-trivial: the gate snapshots only
`plugins/<name>` and `wit/v0`, so an in-repo path dependency on a shared
core fails with `test_rc=125` (we verified this empirically before
publishing). Consuming the core from crates.io is what makes a shared-core
suite pass this gate at all.

The complete upstream workflow ("Validate plugin repository": fmt,
registry contract, WIT drift, component matrix over the full plugin sweep,
package dry run, required gate) also ran green end to end on this branch:
https://github.com/Andy00L/zeroclaw-plugins/actions/runs/29884880566

## Live devnet run: the durable-nonce A/B, and the loop closing on itself

Run on 2026-07-22 against `https://api.devnet.solana.com`, with throwaway
devnet keys used only by host-side dev signing tooling (the plugins never
see a key; they emitted unsigned base64).

Setup (all on chain, all inspectable):

- Sender wallet `J77uvNWcs5bPn6TbspvKsajFfdgYvsfB6PtRhs8ngZZz`, recipient
  `5eJZbddhbcb8QKwRc8C5yofe1cV1vyr9x29dMPULvxrj`.
- Durable nonce account
  [`A5nHMFLBBwFGiaHY6wjQ4sdmcAnXqAT7YBzMo76Su5TZ`](https://explorer.solana.com/address/A5nHMFLBBwFGiaHY6wjQ4sdmcAnXqAT7YBzMo76Su5TZ?cluster=devnet),
  initial nonce blockhash `7VY5aiX2scwt24eMKqE4St9PbEbmaUo2zLHf9zELhgUP`.
- Test mint
  [`FeU3KQR1pxg5jRG8sDB56xx8YdPKzJaYNBFeXBE55MYp`](https://explorer.solana.com/address/FeU3KQR1pxg5jRG8sDB56xx8YdPKzJaYNBFeXBE55MYp?cluster=devnet)
  ("DEMO", 6 decimals), 1000 minted to the sender.

The A/B: `spl_transfer_build`'s core, running against live devnet state
(real mint inspection, real ATA existence probes, real nonce fetch), built
two unsigned 5-DEMO transfers to the allowlisted recipient at the same
moment: one with a recent blockhash, one with the durable nonce. Both were
signed offline and submitted 292 seconds later:

- Recent-blockhash control, REFUSED by the network:
  `{"code":-32002,"message":"Transaction simulation failed: Blockhash not
  found","data":{"err":"BlockhashNotFound",...}}`. This is the structural
  problem the bounty brief names for approval-gated agent payments.
- Durable-nonce transaction, same age, ACCEPTED and finalized:
  [`JyBLEPyLrWhuYAcqvwKasK4WCKMuwNCTSWM3kcNwwkwSsGUantDZsFaf2eV2P9UqDpPjeHHHvxXNxaKpe1LFf81`](https://explorer.solana.com/tx/JyBLEPyLrWhuYAcqvwKasK4WCKMuwNCTSWM3kcNwwkwSsGUantDZsFaf2eV2P9UqDpPjeHHHvxXNxaKpe1LFf81?cluster=devnet)
  (slot 477989374, err null). The transaction carried the
  create-ATA-idempotent, memo, and transfer-checked instructions the
  plugin built; the recipient's balance moved 0 to 5 DEMO; and the nonce
  advanced to `DWHFfGRutFaM9sCgCrQTqLvRYA43yEKtEysJbLzBXhP2`, so the
  signed bytes can never replay.

Then the suite verified itself: `payment_watch`'s core, pointed at the
same devnet RPC, reported the settlement independently:

```
Payment status: PAID
Expected: 5 DEMO to 5eJZbddhbcb8QKwRc8C5yofe1cV1vyr9x29dMPULvxrj
Received: 5 DEMO across 1 settling transaction(s)
  JyBLEPyLrWhuYAcqvwKasK4WCKMuwNCTSWM3kcNwwkwSsGUantDZsFaf2eV2P9UqDpPjeHHHvxXNxaKpe1LFf81 (slot 477989374): +5 DEMO
```

Built by one plugin, finalized by the network, verified by another plugin:
the loop, closed, on a public cluster anyone can inspect.

## In progress (not yet evidence)

Planned artifacts tracked in PUSH_FURTHER.md, listed here so nothing reads
as more than it is: the in-host Telegram approval-gate transcript and the
3-minute demo video.
