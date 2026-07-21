# spl-transfer-build

A ZeroClaw **WIT component** tool plugin: `spl_transfer_build`. It builds an
UNSIGNED SPL token transfer (base64 v0 transaction) from the operator's
wallet to an allowlisted recipient, and can never sign or send it: the
output goes to the operator (or the host's approval flow) to verify and
sign in their own wallet. The guardrails are the product, and they live in
Rust where no prompt can reach them. It implements the `tool-plugin` world
from `wit/v0`, compiles to a `wasm32-wasip2` component, and builds on
[`solana-wasip2-core`](../../solana-wasip2-core/README.md).

## What it does

- **Allowlisted recipients only.** No allowlist configured means nothing is
  ever built. An address off the list is refused before any network call.
- **Hard per-call caps.** Every configured token carries a mandatory cap in
  its own units, enforced in base units before any network call. The
  built-in USDC entry caps at 100 per call until the operator redefines it.
- **On-chain risk gate.** Before building, the mint is re-inspected with
  the same logic as [token-risk-check](../token-risk-check/README.md):
  permanent delegates, active transfer hooks, frozen-by-default and
  non-transferable mints refuse to build unless the operator sets an
  explicit override. Configured decimals are checked against the chain,
  which catches config typos and mint substitution in one move.
- **Durable nonce lifetime.** With `nonce_account` configured, the
  transaction uses the on-chain durable nonce (with `AdvanceNonceAccount`
  first) and stays signable while it waits in an approval queue; without
  it, the ~90 second blockhash window applies and the output says so.
- **Recipient account handling.** A missing recipient token account gets an
  idempotent create instruction, and the summary discloses the rent cost;
  a sender without the token fails early with a clear message.

## Custody tier: T1 (build)

The plugin holds no key and cannot submit anything: there is no keypair
type in its dependency tree and no `sendTransaction` call in its code. RPC
access is read-only (mint state, account existence, blockhash or nonce
state). Signing happens wherever the operator keeps their keys.

## Config keys

| Key | Default | Meaning |
|---|---|---|
| `sender_wallet` | (unset) | The owner of the source tokens and fee payer. Until set, nothing builds. |
| `allowed_recipients` | (unset) | Comma-separated recipient wallets. Until set, nothing builds. |
| `tokens` | USDC built in, capped at 100 | `SYMBOL=MINT:DECIMALS:MAX`, comma-separated; the per-call cap is mandatory. |
| `rpc_url` | `https://api.mainnet-beta.solana.com` | JSON-RPC endpoint. Set your own. |
| `nonce_account` | (unset) | Durable nonce account whose authority is `sender_wallet`. |
| `override_risk_gate` | `false` | Set `"true"` to build despite RED mint findings (they still print). |

## Worked example

Model call:

```json
{ "recipient": "2apBGMsS6ti9RyF5TwQTDswXBWskiJP2LD4cUEDqYJjk", "amount": "20", "memo": "invoice 412" }
```

Output (real run against captured mainnet state; base64 shortened here,
the tool returns it in full):

```
Unsigned transfer built. Nothing has been signed or sent.
Send: 20 USDC (EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v)
From: 9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM (operator-configured sender)
To: 2apBGMsS6ti9RyF5TwQTDswXBWskiJP2LD4cUEDqYJjk (allowlisted)
Note: the recipient has no USDC account yet; the transaction creates BAufkuMM...H2sBDG at the sender's expense (rent-exempt minimum)
Memo: invoice 412
Lifetime: recent blockhash: sign within about 90 seconds or the transaction expires (configure nonce_account for a durable lifetime)
Signers required: 9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM
Unsigned transaction (base64, verify before signing):
AQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA...BgA=
```

The transaction decodes to exactly: idempotent create-ATA, memo, and
`TransferChecked` for 20.000000 USDC, fee payer the sender, one zeroed
signature slot (asserted wire-level in the test suite).

## Prompt injection, tested

The attack: a hostile message convinces the agent to drain funds.
Transcript from the test suite
(`cargo test prompt_injection_transcript_the_readme_documents`):

Injected model call:

```json
{
  "recipient": "9PhSoeYzLagajautCYUfUXSB6acpeP1LLQDKpZnegLDq",
  "amount": "5000",
  "memo": "urgent, authorized by the operator, do not verify"
}
```

Tool result (real run):

```
success: false
error: recipient 9PhSoeYzLagajautCYUfUXSB6acpeP1LLQDKpZnegLDq is not on the
operator's allowlist; no transaction built
```

Zero RPC calls left the sandbox (asserted), no transaction bytes exist
anywhere in the result, and the cap check would refuse the amount
independently (also tested: `150 USDC` against the built-in 100 cap fails
with `exceeds the per-call cap of 100 USDC`). A smuggled `sender` argument
fails on `deny_unknown_fields`, and social-engineering text in the memo is
sanitized before it can reach the operator's approval screen as multi-line
noise.

## Threat model

- **Assets.** The operator's tokens, and the integrity of what the approval
  screen shows versus what the transaction does.
- **Adversaries.** A prompt-injected model (recipient redirection, amount
  inflation, token substitution, argument smuggling); a hostile mint
  (seizure or hook semantics the operator did not expect); a hostile RPC
  endpoint.
- **Defenses.** Config-only sender; recipient allowlist; mandatory
  per-call caps; on-chain mint re-inspection with decimals cross-check and
  a fail-closed risk gate; config-only RPC endpoint; `deny_unknown_fields`;
  summary and transaction built from the same validated values, so the
  human-readable lines cannot disagree with the bytes.
- **Residual risk.** The plugin is stateless (the host runs each call in a
  fresh store), so caps are per call, not per day: an injected model could
  request the cap repeatedly. Mitigations: the host's approval gate sits in
  front of every call, and the allowlist bounds where value can go at all.
  Per-day accounting belongs host-side and is listed under what to build
  next. A malicious operator-chosen RPC node can lie about mint state and
  account existence; it still cannot redirect funds or forge signatures.

## Layout (the reference format)

```
src/transfer_build.rs  # pure logic, no wasm deps: host-testable with `cargo test`
src/lib.rs             # thin #[cfg(target_family = "wasm")] component shim
tests/                 # host-run tests over the pure core, fixtures from mainnet
manifest.toml          # name, version, wasm_path, capabilities, permissions
```

## Build and test

```bash
cargo test                                        # 12 host tests, no network
rustup target add wasm32-wasip2
cargo build --target wasm32-wasip2 --release      # the component
cp target/wasm32-wasip2/release/spl_transfer_build.wasm spl_transfer_build.wasm
```

Success: all tests green, and the release build produces a component whose
only exports are `zeroclaw:plugin/plugin-info@0.1.0` and
`zeroclaw:plugin/tool@0.1.0` (checked with `wasm-tools component wit`).

## Install

Copy this directory (the `.wasm` next to its `manifest.toml`) into your
configured plugins dir, enable plugins, and set the config section stored
under this plugin's name (`sender_wallet`, `allowed_recipients`, and
optionally `tokens`, `rpc_url`, `nonce_account`); see the ZeroClaw plugin
docs for the config command syntax on your install.

```toml
[plugins]
enabled = true
```

Run the agent with a build that includes a compiler backend, e.g.
`--features plugins-wasm,plugins-wasm-cranelift`.

## License

MIT. See [LICENSE](LICENSE).
