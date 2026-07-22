//! Host tests for the operator token symbol map: the built-in entries, the
//! `SYMBOL=MINT:DECIMALS` config syntax, and the override semantics payment
//! plugins inherit.

use solana_wasip2_core::token_map::{
    built_in_symbol_map, extend_symbol_map_from_config, parse_symbol_token_definition, USDC_MINT,
};

/// PYUSD mainnet mint, reused as an arbitrary valid mint in these tests.
const PYUSD_MINT: &str = "2b1kV6DkPAnxd5ixfnxCpjxmKwqjjaYmCZfHsFu24GXo";

#[test]
fn the_built_in_map_holds_usdc_and_native_sol() {
    let symbol_map = built_in_symbol_map();
    let usdc_entry = symbol_map.get("USDC").expect("USDC must be built in");
    assert_eq!(usdc_entry.mint.unwrap().to_string(), USDC_MINT);
    assert_eq!(usdc_entry.decimals, 6);
    let sol_entry = symbol_map.get("SOL").expect("SOL must be built in");
    assert!(sol_entry.mint.is_none(), "native SOL has no mint");
    assert_eq!(sol_entry.decimals, 9);
}

#[test]
fn symbols_are_uppercased_and_fields_tolerate_whitespace() {
    let (symbol, entry) =
        parse_symbol_token_definition(&format!("  pyusd = {PYUSD_MINT} : 6 ")).unwrap();
    assert_eq!(symbol, "PYUSD");
    assert_eq!(entry.mint.unwrap().to_string(), PYUSD_MINT);
    assert_eq!(entry.decimals, 6);
}

#[test]
fn operators_can_override_a_built_in_entry() {
    // Redefining USDC is the operator's right (config is trusted); the last
    // definition wins, exactly like any other config override.
    let mut symbol_map = built_in_symbol_map();
    extend_symbol_map_from_config(&mut symbol_map, &format!("USDC={PYUSD_MINT}:6")).unwrap();
    assert_eq!(
        symbol_map.get("USDC").unwrap().mint.unwrap().to_string(),
        PYUSD_MINT
    );
}

#[test]
fn a_trailing_comma_fails_closed_with_the_syntax_message() {
    // "A=..:6," splits into a valid entry plus an empty one; the empty entry
    // is a config mistake and must be named, not silently dropped.
    let mut symbol_map = built_in_symbol_map();
    let config_error =
        extend_symbol_map_from_config(&mut symbol_map, &format!("PYUSD={PYUSD_MINT}:6,"))
            .unwrap_err();
    assert!(
        config_error.contains("must look like SYMBOL=MINT:DECIMALS"),
        "got: {config_error}"
    );
}
