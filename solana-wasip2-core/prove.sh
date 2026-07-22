#!/usr/bin/env bash
# One-command proof for the Solana suite: host tests, lints on both targets,
# wasm component builds, and the cross-stack transaction oracle.
# Run from solana-wasip2-core/:  ./prove.sh
# Exits 0 only when every check passes. Needs: Rust with wasm32-wasip2, node.
set -euo pipefail

SUITE_CRATES=(
  "."
  "../plugins/token-risk-check"
  "../plugins/solana-pay-request"
  "../plugins/spl-transfer-build"
  "../plugins/payment-watch"
)

total_tests=0
for crate_dir in "${SUITE_CRATES[@]}"; do
  crate_name=$(basename "$(cd "$crate_dir" && pwd)")
  echo "==> $crate_name: cargo test"
  test_output=$(cd "$crate_dir" && cargo test 2>&1)
  crate_tests=$(echo "$test_output" | grep -oE '[0-9]+ passed' | awk '{s+=$1} END {print s}')
  if echo "$test_output" | grep -qE 'FAILED|error\['; then
    echo "$test_output" | tail -20
    echo "FAIL: $crate_name tests"
    exit 1
  fi
  total_tests=$((total_tests + crate_tests))
  echo "    $crate_tests passed"

  echo "==> $crate_name: clippy (host + wasm32-wasip2, -D warnings)"
  (cd "$crate_dir" && cargo clippy --all-targets -- -D warnings)
  (cd "$crate_dir" && cargo clippy --target wasm32-wasip2 -- -D warnings)

  if [ "$crate_dir" != "." ]; then
    echo "==> $crate_name: wasm32-wasip2 release build"
    (cd "$crate_dir" && cargo build --target wasm32-wasip2 --release)
  else
    echo "==> $crate_name: wasm32-wasip2 check"
    (cd "$crate_dir" && cargo check --target wasm32-wasip2)
  fi
done

echo "==> cross-stack oracle (vs @solana/web3.js)"
cargo run --quiet --example emit_oracle_vectors > tools/byte-oracle/vectors.json
(cd tools/byte-oracle && npm install --silent --no-audit --no-fund && node check.mjs)

echo ""
echo "ALL PROOFS PASSED: $total_tests host tests, clippy clean on both targets,"
echo "4 wasm components built, oracle semantically identical to web3.js."
