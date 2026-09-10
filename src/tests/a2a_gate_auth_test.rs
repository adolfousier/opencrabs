//! The a2a gateway must never come up as an open, unauthenticated gate, and its
//! bearer check must not leak the token through comparison timing (#1473).
//!
//! `send`/`stream` run the agent with its full tool surface for any caller that
//! clears the bearer middleware, and that middleware allows everyone when no
//! key is set. Loopback is safe; any other bind with no key is an open gate.

use crate::a2a::server::{check_gate_authenticated, tokens_match};

// ── the startup guard ───────────────────────────────────────────────────────

#[test]
fn loopback_without_a_key_is_allowed() {
    // Same-box callers only; this is the safe default posture.
    assert!(check_gate_authenticated("127.0.0.1", None).is_ok());
    assert!(check_gate_authenticated("::1", None).is_ok());
}

#[test]
fn any_key_makes_any_bind_allowed() {
    // A configured key is the authorization boundary, so the bind is free.
    assert!(check_gate_authenticated("0.0.0.0", Some("secret")).is_ok());
    assert!(check_gate_authenticated("192.168.1.10", Some("secret")).is_ok());
}

#[test]
fn a_wildcard_bind_without_a_key_is_refused() {
    // 0.0.0.0 is every interface: the exact open-gate case.
    let err = check_gate_authenticated("0.0.0.0", None).unwrap_err();
    assert!(
        err.contains("not loopback"),
        "message names the cause: {err}"
    );
    assert!(err.contains("api_key"), "message says how to fix it: {err}");
}

#[test]
fn a_public_ip_without_a_key_is_refused() {
    assert!(check_gate_authenticated("192.168.1.10", None).is_err());
    assert!(check_gate_authenticated("10.0.0.5", None).is_err());
}

#[test]
fn a_non_ip_bind_without_a_key_fails_safe() {
    // A hostname never parses to an IpAddr (and never into the SocketAddr the
    // server needs), so it is treated as not-loopback and refused rather than
    // assumed safe.
    assert!(check_gate_authenticated("localhost", None).is_err());
    assert!(check_gate_authenticated("", None).is_err());
}

// ── constant-time-equivalent bearer comparison ──────────────────────────────

#[test]
fn a_matching_token_authorizes() {
    assert!(tokens_match("the-secret-token", "the-secret-token"));
}

#[test]
fn a_wrong_token_is_rejected() {
    assert!(!tokens_match("wrong", "the-secret-token"));
    // A correct prefix must not pass: this is exactly the byte-walk the
    // hash-then-compare defeats.
    assert!(!tokens_match("the-secret-toke", "the-secret-token"));
    assert!(!tokens_match("the-secret-tokenX", "the-secret-token"));
}

#[test]
fn an_empty_presented_token_is_rejected() {
    assert!(!tokens_match("", "the-secret-token"));
}
