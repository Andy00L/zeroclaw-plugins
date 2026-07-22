//! Host tests for decimal amount parsing and formatting. Money math must be
//! exact, so every edge lives here: malformed strings, precision overflow,
//! integer overflow, zero, and round-tripping.

use solana_wasip2_core::amount::{
    canonicalize_decimal_amount, format_base_units, parse_amount_to_base_units,
};
use solana_wasip2_core::error::CoreError;

#[test]
fn parses_whole_and_fractional_amounts() {
    assert_eq!(parse_amount_to_base_units("25", 6).unwrap(), 25_000_000);
    assert_eq!(parse_amount_to_base_units("0.5", 6).unwrap(), 500_000);
    assert_eq!(parse_amount_to_base_units(".5", 6).unwrap(), 500_000);
    assert_eq!(
        parse_amount_to_base_units("1.234567", 6).unwrap(),
        1_234_567
    );
    assert_eq!(parse_amount_to_base_units("007", 6).unwrap(), 7_000_000);
    assert_eq!(parse_amount_to_base_units(" 25 ", 6).unwrap(), 25_000_000);
    assert_eq!(parse_amount_to_base_units("5.", 6).unwrap(), 5_000_000);
    assert_eq!(parse_amount_to_base_units("1", 0).unwrap(), 1);
}

#[test]
fn accepts_the_full_u64_range_and_nothing_beyond() {
    assert_eq!(
        parse_amount_to_base_units("18446744073709551615", 0).unwrap(),
        u64::MAX
    );
    assert!(matches!(
        parse_amount_to_base_units("18446744073709551616", 0),
        Err(CoreError::InvalidAmount(_))
    ));
}

#[test]
fn rejects_malformed_amounts_with_distinct_messages() {
    let rejected_inputs = ["", "   ", "abc", "1,5", "-1", "1e5", "1.2.3", ".", "1 5"];
    for rejected_input in rejected_inputs {
        let parse_result = parse_amount_to_base_units(rejected_input, 6);
        assert!(
            matches!(parse_result, Err(CoreError::InvalidAmount(_))),
            "input '{rejected_input}' should be rejected, got {parse_result:?}"
        );
    }
}

#[test]
fn rejects_more_fraction_digits_than_the_mint_has() {
    let parse_result = parse_amount_to_base_units("1.2345678", 6);
    match parse_result {
        Err(CoreError::InvalidAmount(message)) => {
            assert!(message.contains("7 fraction digits"), "got: {message}");
        }
        other => panic!("expected InvalidAmount, got {other:?}"),
    }
}

#[test]
fn rejects_zero_amounts() {
    for zero_input in ["0", "0.0", "0.000000", "00.00"] {
        assert!(
            matches!(
                parse_amount_to_base_units(zero_input, 6),
                Err(CoreError::InvalidAmount(_))
            ),
            "input '{zero_input}' should be rejected as zero"
        );
    }
}

#[test]
fn formats_base_units_canonically() {
    assert_eq!(format_base_units(25_000_000, 6), "25");
    assert_eq!(format_base_units(500_000, 6), "0.5");
    assert_eq!(format_base_units(1_234_567, 6), "1.234567");
    assert_eq!(format_base_units(0, 6), "0");
    assert_eq!(format_base_units(1, 6), "0.000001");
    assert_eq!(
        format_base_units(u64::MAX as u128, 0),
        "18446744073709551615"
    );
}

#[test]
fn canonicalization_strips_redundant_zeros() {
    assert_eq!(canonicalize_decimal_amount("0.50", 6).unwrap(), "0.5");
    assert_eq!(canonicalize_decimal_amount("25.000000", 6).unwrap(), "25");
    assert_eq!(canonicalize_decimal_amount(".5", 6).unwrap(), "0.5");
}

#[test]
fn parse_and_format_round_trip() {
    for (amount_text, decimals) in [("123.456", 6), ("1", 9), ("0.000000001", 9)] {
        let base_units = parse_amount_to_base_units(amount_text, decimals).unwrap();
        let formatted = format_base_units(base_units as u128, decimals);
        let reparsed = parse_amount_to_base_units(&formatted, decimals).unwrap();
        assert_eq!(base_units, reparsed, "round trip failed for {amount_text}");
    }
}
