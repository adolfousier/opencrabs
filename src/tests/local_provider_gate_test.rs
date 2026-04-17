//! Tests for local-provider detection — the gate used to decide whether
//! to inject `chat_template_kwargs: {enable_thinking: X}` into outgoing
//! request bodies.
//!
//! The gate has to be tight: injecting `chat_template_kwargs` into a
//! cloud request (DashScope, OpenRouter, etc.) gets either silently
//! ignored or rejected, and we want the transform to fire for every
//! self-hosted llama.cpp / MLX / LM Studio / Ollama endpoint the user
//! might run — loopback, mDNS, or LAN. These tests lock the boundary.

use crate::brain::provider::factory::is_local_base_url;

#[test]
fn detects_loopback_hosts() {
    assert!(is_local_base_url(
        "http://localhost:1234/v1/chat/completions"
    ));
    assert!(is_local_base_url("http://127.0.0.1:8080"));
    assert!(is_local_base_url("http://[::1]:1234/v1"));
    assert!(is_local_base_url("http://0.0.0.0:11434"));
}

#[test]
fn detects_mdns_local() {
    assert!(is_local_base_url("http://mac-studio.local:1234"));
    assert!(is_local_base_url("http://server.local/v1"));
}

#[test]
fn detects_rfc1918_private() {
    assert!(is_local_base_url("http://192.168.1.5:1234"));
    assert!(is_local_base_url("http://10.0.0.12:8000"));
    assert!(is_local_base_url("http://172.16.0.1:1234"));
    assert!(is_local_base_url("http://172.31.255.254:1234"));
}

#[test]
fn rejects_public_hosts() {
    assert!(!is_local_base_url(
        "https://api.openai.com/v1/chat/completions"
    ));
    assert!(!is_local_base_url("https://openrouter.ai/api/v1"));
    assert!(!is_local_base_url("https://dashscope.aliyuncs.com/api/v1"));
    assert!(!is_local_base_url("https://api.anthropic.com"));
    // 172.15.x.x and 172.32.x.x are OUTSIDE the RFC1918 range.
    assert!(!is_local_base_url("http://172.15.0.1"));
    assert!(!is_local_base_url("http://172.32.0.1"));
    // 11.x.x.x looks like 10.x.x.x but isn't private.
    assert!(!is_local_base_url("http://11.0.0.1"));
}

#[test]
fn case_insensitive_host_match() {
    assert!(is_local_base_url("http://LOCALHOST:1234"));
    assert!(is_local_base_url("http://My-Mac.LOCAL:1234"));
}

#[test]
fn handles_missing_scheme() {
    assert!(is_local_base_url("localhost:1234"));
    assert!(is_local_base_url("127.0.0.1"));
}
