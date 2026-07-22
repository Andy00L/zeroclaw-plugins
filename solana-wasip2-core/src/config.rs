//! Config-section hygiene shared by every plugin.
//!
//! The host injects a flat string map under `__config`. A typoed key
//! (`allowed_recipient` instead of `allowed_recipients`) that is silently
//! ignored turns a guardrail off without anyone noticing, which is a
//! fail-open failure. Plugins built on this crate refuse unknown keys
//! instead: the operator learns about the typo on the first call.

use std::collections::HashMap;

/// Return the config keys that are not in the accepted set, sorted, so the
/// caller can fail closed with a message naming them. An empty result means
/// the section is clean.
pub fn find_unknown_config_keys(
    section: &HashMap<String, String>,
    accepted_keys: &[&str],
) -> Vec<String> {
    let mut unknown_keys: Vec<String> = section
        .keys()
        .filter(|present_key| !accepted_keys.contains(&present_key.as_str()))
        .cloned()
        .collect();
    unknown_keys.sort_unstable();
    unknown_keys
}

/// Standard fail-closed message for unknown config keys, shared so all
/// plugins report the same way.
pub fn describe_unknown_config_keys(unknown_keys: &[String], accepted_keys: &[&str]) -> String {
    format!(
        "config error: unknown key(s) {}; accepted keys: {}. Refusing to run with a \
         possibly mistyped configuration",
        unknown_keys.join(", "),
        accepted_keys.join(", ")
    )
}
