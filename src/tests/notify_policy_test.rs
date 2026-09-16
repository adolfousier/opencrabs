//! Tests for the v2 notify delivery policy sender-label validation.
//!
//! Extracted from an inline `#[cfg(test)]` block in
//! `src/brain/agent/service/notify_policy.rs`; project policy
//! (CONTRIBUTING.md) requires all tests under `src/tests/`.

use crate::brain::agent::service::notify_policy::{SENDER_LABEL_MAX_CHARS, validate_sender_label};

#[test]
fn sender_label_rejects_framing_breakers_and_overlong() {
    assert!(validate_sender_label("ok label").is_ok());
    let brk = validate_sender_label("bad]label").unwrap_err();
    assert!(brk.contains("must not contain"), "got: {brk}");
    let nl = validate_sender_label("bad\nlabel").unwrap_err();
    assert!(nl.contains("must not contain"), "got: {nl}");
    let long = validate_sender_label("x".repeat(SENDER_LABEL_MAX_CHARS + 1).as_str()).unwrap_err();
    assert!(long.contains("at most"), "got: {long}");
    assert!(validate_sender_label("x".repeat(SENDER_LABEL_MAX_CHARS).as_str()).is_ok());
}
