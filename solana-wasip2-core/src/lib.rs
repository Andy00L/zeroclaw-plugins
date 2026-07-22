//! Solana primitives for wasm32-wasip2 WebAssembly components.
//!
//! Pure-core substrate for ZeroClaw tool plugins: a JSON-RPC client over a
//! mockable blocking HTTP trait (backed by `waki`/`wasi:http` inside a
//! component), unsigned v0 transaction construction, durable nonce parsing,
//! Solana Pay transfer request URLs, decimal amount math without floating
//! point, and token mint risk inspection.
//!
//! Design rules, matching the zeroclaw-plugins reference plugin layout:
//! nothing in this crate requires a wasm toolchain to test (`cargo test`
//! runs on the host with a mocked transport), no private key ever enters
//! this crate, and every fallible function returns a typed [`error::CoreError`]
//! instead of panicking.

pub mod addresses;
pub mod amount;
pub mod config;
pub mod error;
pub mod http;
pub mod mint_inspect;
pub mod nonce;
pub mod pay_url;
pub mod payment_verify;
pub mod rpc;
pub mod shape;
pub mod token_map;
pub mod txbuild;
