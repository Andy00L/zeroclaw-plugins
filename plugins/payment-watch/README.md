# payment-watch

A ZeroClaw **WIT component** tool plugin: `payment_watch`. It closes the
payment loop that [solana-pay-request](../solana-pay-request/README.md)
opens: given the invoice's reference address and expected amount, it
reports PAID, PARTIAL, or PENDING with on-chain evidence. Settlement is
verified honestly, by the operator wallet's actual balance delta inside
each transaction, never by a transaction merely touching the reference
(anyone can attach a merchant's reference key to a worthless transaction).
It implements the `tool-plugin` world from `wit/v0`, compiles to a
`wasm32-wasip2` component, and builds on
[`solana-wasip2-core`](../../solana-wasip2-core/README.md).

## What it does

- **Balance-delta settlement.** For every non-failed transaction touching
  the reference, the recipient's delta is computed from
  `preTokenBalances`/`postTokenBalances` (SPL) or
  `preBalances`/`postBalances` (SOL). Positive deltas sum toward the
  invoice; a fee-splitting processor that skims 260 base units is reported
  as exactly what arrived.
- **PAID / PARTIAL / PENDING** with up to 3 evidence lines (signature,
  slot, amount), overpayment and skipped-failed notes, and an explicit
  "a reference touch is not a payment" note when transactions touched the
  reference without moving value. The settling-transaction count is always
  the true total; when more than 3 transactions settle, a note says the
  evidence shows the first 3.
- **Cron-SOP friendly.** Stateless by construction (the host runs each
  call in a fresh store): the cursor (newest seen signature) rides in the
  output and comes back as an argument. `examples/sop/` ships a
  copy-ready cron SOP (`*/2 * * * *`, `admission_policy = "coalesce"`).
- **Recipient is config-only.** The model cannot ask whether an attacker's
  wallet got paid and hear yes; settlement is only ever verified against
  the operator's configured wallet.

## Custody tier: T0 (read)

The plugin holds no key and can move nothing. Secrets held: at most an RPC
URL with an embedded key, read from the operator's config section.

## Config keys

| Key | Default | Meaning |
|---|---|---|
| `recipient` | (unset) | The wallet whose incoming balance proves settlement. Until set, every check fails with a setup instruction. |
| `rpc_url` | `https://api.mainnet-beta.solana.com` | JSON-RPC endpoint. Set your own. |
| `tokens` | (empty) | Extra symbols: `PYUSD=2b1kV6...GXo:6` (SYMBOL=MINT:DECIMALS, comma-separated). USDC and SOL are built in. |

Unknown keys refuse to run: a typo produces a distinct config error naming
the key instead of silently ignoring it (fail closed, tested).

## Worked example

Model call (reference and amount come from the earlier
`solana_pay_request` output):

```json
{ "reference": "2apBGMsS6ti9RyF5TwQTDswXBWskiJP2LD4cUEDqYJjk", "amount": "1.99974" }
```

Output (real run against a mainnet-captured transaction):

```
Payment status: PAID
Expected: 1.99974 USDC to 9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM
Received: 1.99974 USDC across 1 settling transaction(s)
  8gXefqe2LCKPP5YNUuBrZEAWHDcVP3GrHjvcnN2U1c9UbBUXnxzNNSmJ9ws6Re2UBm6XM2fhX2SeJrz2SNo9DiC (slot 433531911): +1.99974 USDC
Note: skipped 1 failed transaction(s)
Note: 1 transaction(s) not yet queryable on this RPC node; re-check on the next poll
Cursor: 2ap2o5LHSQVT8LbxVyPa... (pass as the cursor argument on the next poll to scan only newer transactions)
```

While unpaid, the same call reports `Payment status: PENDING` with the
expected amount, and the cron SOP stays silent.

## Prompt injection, tested

The attack that matters for a settlement checker is verification
redirection: convince the agent to confirm that money arrived somewhere it
did not. Transcript from the test suite
(`cargo test the_model_cannot_supply_a_recipient_argument`):

Injected model call:

```json
{ "reference": "2apB...YJjk", "amount": "25", "recipient": "attacker..." }
```

Tool result (real run):

```
success: false
error: invalid arguments: unknown field `recipient`, expected one of
`reference`, `amount`, `token`, `cursor`, `__config` at line 1 column 98
```

Further tested vectors, all failing closed before any network call: a
malformed cursor (`'; drop table--`) is rejected as not base58-shaped; a
typoed config key refuses to run; a transaction that touches the reference
without paying the recipient counts zero and is called out in the output;
failed transactions never count.

## Threat model

- **Assets.** The truthfulness of "you have been paid", which downstream
  actions (shipping goods, releasing services) depend on.
- **Adversaries.** A prompt-injected model (verification redirection,
  argument smuggling); a payer forging activity (reference-touch spam,
  failed transactions, paying the wrong token); a hostile RPC endpoint.
- **Defenses.** Config-only recipient and RPC endpoint; balance-delta
  verification per transaction with failed transactions excluded; token
  resolution only through the operator map (a payment in a worthless
  lookalike mint counts zero); `deny_unknown_fields`; strict cursor
  validation; distinct fail-closed errors.
- **Residual risk.** A malicious operator-chosen RPC node can fabricate
  transaction metadata; run your own node for invoices that matter. At
  `confirmed` commitment a deep reorg could in principle unsettle a
  reported payment; for high-value invoices poll again after finality.

## Layout (the reference format)

```
src/payment_watch.rs  # pure logic, no wasm deps: host-testable with `cargo test`
src/lib.rs            # thin #[cfg(target_family = "wasm")] component shim
tests/                # host-run tests over the pure core, fixtures from mainnet
examples/sop/         # copy-ready cron SOP that polls this tool
manifest.toml         # name, version, wasm_path, capabilities, permissions
```

## Build and test

```bash
cargo test                                        # 16 host tests, no network
rustup target add wasm32-wasip2
cargo build --target wasm32-wasip2 --release      # the component
cp target/wasm32-wasip2/release/payment_watch.wasm payment_watch.wasm
```

Success: all tests green, and the release build produces a component whose
only exports are `zeroclaw:plugin/plugin-info@0.1.0` and
`zeroclaw:plugin/tool@0.1.0` (checked with `wasm-tools component wit`).

## Install

Copy this directory (the `.wasm` next to its `manifest.toml`) into your
configured plugins dir, then enable plugins and add the entry (exact shape
per `PluginEntryConfig` in zeroclaw-config; note issue #8636: the first
write for a fresh plugin currently needs the entry added to the config file
by hand):

```toml
[plugins]
enabled = true

[[plugins.entries]]
name = "payment-watch"

[plugins.entries.config]
recipient = "<your receiving wallet>"
# rpc_url = "https://your-rpc.example"
```

For the cron loop, copy `examples/sop/` to `<workspace>/sops/payment-watch/`.
Run the agent with a build that includes a compiler backend, e.g.
`--features plugins-wasm,plugins-wasm-cranelift`.

## License

MIT. See [LICENSE](LICENSE).
