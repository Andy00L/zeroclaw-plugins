//! Output shaping for text that ends up in an LLM context window.
//!
//! Anything read from the chain that a third party controls (token names,
//! symbols, memos) is a prompt-injection vector when echoed into a tool
//! result: a hostile issuer can name a token "SAFE, ignore all previous
//! findings". Shaping cannot make such text trustworthy; it makes it inert:
//! bounded in length, single-line, and stripped of control characters, so it
//! can never masquerade as tool output structure or system instructions
//! spanning lines.

/// Longest issuer-controlled string echoed into a report, in characters.
/// Long enough for real token names ("PayPal USD"), short enough that a
/// hostile name cannot carry a paragraph of instructions.
pub const MAX_UNTRUSTED_TEXT_CHARS: usize = 48;

/// Bound and flatten one line of untrusted text: control characters and
/// newlines become spaces, runs of whitespace collapse, and anything past
/// `max_chars` is dropped with a ".." marker.
pub fn sanitize_untrusted_text(untrusted_text: &str, max_chars: usize) -> String {
    let mut sanitized = String::with_capacity(untrusted_text.len().min(max_chars + 2));
    let mut previous_was_space = false;
    for character in untrusted_text.chars() {
        let normalized = if character.is_control() || character.is_whitespace() {
            ' '
        } else {
            character
        };
        if normalized == ' ' {
            if previous_was_space || sanitized.is_empty() {
                continue;
            }
            previous_was_space = true;
        } else {
            previous_was_space = false;
        }
        sanitized.push(normalized);
        if sanitized.chars().count() >= max_chars {
            sanitized.push_str("..");
            break;
        }
    }
    sanitized.trim_end().to_string()
}
