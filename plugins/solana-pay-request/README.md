# solana-pay-request

A ZeroClaw **WIT component** tool plugin: `solana_pay_request`. It turns
"charge table 4 for 25 USDC" into a spec-conformant
[Solana Pay](https://docs.solanapay.com/spec) transfer request URL with a
fresh reference key for confirming the payment. The receiving wallet is
operator-configured only: the tool has no recipient argument, so a hijacked
model cannot redirect funds. It implements the `tool-plugin` world from
`wit/v0`, compiles to a `wasm32-wasip2` component, and builds on
[`solana-wasip2-core`](../../solana-wasip2-core/README.md).

## What it does

- **Payment terminal semantics.** Every request pays the operator's
  configured wallet. Amount, token symbol, label, message, and memo are the
  model's inputs; the destination never is.
- **Token symbol map.** USDC (mainnet mint, 6 decimals) and native SOL are
  built in; operators add symbols via config. Raw mint addresses from the
  model are never accepted.
- **Per-payment reference.** Each URL carries a fresh random reference key
  (WASI randomness), returned in the output, so the payment can be found
  later with `getSignaturesForAddress`.
- **Zero secrets, zero network.** The plugin makes no RPC calls and its
  manifest requests only `config_read`. There is nothing in it to steal.

## Custody tier: T1 (build)

The plugin builds a payment request a human pays from their own wallet.
Secrets held: none. Keys held: none. Network access: none.

## Config keys

| Key | Default | Meaning |
|---|---|---|
| `recipient` | (unset) | The receiving wallet. Until set, every request fails with a setup instruction. |
| `tokens` | (empty) | Extra symbols: `PYUSD=2b1kV6...GXo:6,BRL2=...:4` (SYMBOL=MINT:DECIMALS, comma-separated, decimals 9 max). |

Unknown keys refuse to run: a typo produces a distinct config error naming
the key instead of silently ignoring it (fail closed, tested).

## Worked example

Model call:

```json
{ "amount": "25", "label": "Casa Zero", "message": "Table 4 dinner", "memo": "order#412" }
```

Output (real run; reference key varies per call):

```
Payment request created.
Pay URL: solana:9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM?amount=25&spl-token=EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v&reference=4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi&label=Casa%20Zero&message=Table%204%20dinner&memo=order%23412
Amount: 25 USDC
To: 9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM (operator-configured recipient)
Reference: 4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi
Confirm receipt by watching the reference address for the incoming transaction.
```

Any Solana Pay wallet opens the URL (or its QR encoding) and pays into the
configured wallet with the reference attached.

## Prompt injection, tested

The attack that matters for a payment tool is redirection. Transcript from
the test suite (`cargo test the_model_cannot_supply_a_recipient_argument`):

Injected model call:

```json
{ "amount": "25", "recipient": "attacker111...", "__config": { "recipient": "<operator wallet>" } }
```

Tool result (real run):

```
success: false
error: invalid arguments: unknown field `recipient`, expected one of
`amount`, `token`, `label`, `message`, `memo`, `__config` at line 1 column 98
```

Arguments are parsed with `deny_unknown_fields`, so a smuggled recipient is
a loud failure, not a silent ignore. Two more tested vectors: a label of
`shop&amount=999999&recipient=attacker` arrives percent-encoded
(`label=shop%26amount%3D999999%26recipient%3Dattacker`, still exactly one
`amount=` in the URL), and unknown token symbols fail closed listing what
is configured. The host deletes any model-supplied `__config` before
injection, so the config path in this transcript exists only in tests.

## Threat model

- **Assets.** The direction of customer payments and the integrity of the
  payment URL.
- **Adversaries.** A prompt-injected model (redirection, parameter
  smuggling, token substitution); a hostile customer message that becomes a
  label or memo.
- **Defenses.** No recipient argument exists; unknown arguments are
  rejected; tokens resolve only through the operator map; free text is
  sanitized (single line, bounded) and percent-encoded; amounts are
  validated integer math with per-token precision.
- **Residual risk.** The model chooses the amount within the token's
  precision, so an injected message can ask for a wrong amount; the payer
  still sees and approves the real amount in their wallet before signing.

## Layout (the reference format)

```
src/pay_request.rs  # pure logic, no wasm deps: host-testable with `cargo test`
src/lib.rs          # thin #[cfg(target_family = "wasm")] component shim
tests/              # host-run tests over the pure core
manifest.toml       # name, version, wasm_path, capabilities, permissions
```

## Build and test

```bash
cargo test                                        # 10 host tests, no network
rustup target add wasm32-wasip2
cargo build --target wasm32-wasip2 --release      # the component
cp target/wasm32-wasip2/release/solana_pay_request.wasm solana_pay_request.wasm
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
name = "solana-pay-request"

[plugins.entries.config]
recipient = "<your receiving wallet>"
# tokens = "PYUSD=2b1kV6DkPAnxd5ixfnxCpjxmKwqjjaYmCZfHsFu24GXo:6"
```

Run the agent with a build that includes a compiler backend, e.g.
`--features plugins-wasm,plugins-wasm-cranelift`.

## License

MIT. See [LICENSE](LICENSE).
