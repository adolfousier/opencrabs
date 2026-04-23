//! Regression tests for proxy-error surfacing and retry classification.
//!
//! Locks in the fix for the 2026-04-23 incident where opencode.ai/zen/go
//! returned HTTP 400 with `{"error":{"message":"Provider returned error",
//! "metadata":{"raw":"{\"error\":{\"message\":\"thinking is enabled but
//! reasoning_content is missing in assistant tool call message at index
//! 39\",\"type\":\"invalid_request_error\"}},"provider_name":"Moonshot AI"}}}`
//! — the real Moonshot error was hidden inside `metadata.raw` and we were
//! treating every 400 as non-retryable regardless of content.

use crate::brain::provider::custom_openai_compatible::{
    OpenAIErrorResponse, needs_reasoning_content_for, unwrap_proxy_error,
};
use crate::brain::provider::error::{ProviderError, is_transient_proxy_400};

// ─── unwrap_proxy_error ─────────────────────────────────────────────

#[test]
fn unwrap_proxy_error_pulls_inner_message_from_opencode_envelope() {
    let body = r#"{
      "error": {
        "message": "Provider returned error",
        "code": 400,
        "metadata": {
          "raw": "{\"error\":{\"message\":\"thinking is enabled but reasoning_content is missing in assistant tool call message at index 39\",\"type\":\"invalid_request_error\"}}",
          "provider_name": "Moonshot AI",
          "is_byok": true
        }
      },
      "user_id": "user_x"
    }"#;
    let parsed: OpenAIErrorResponse = serde_json::from_str(body).expect("parse");
    let (msg, ty) = unwrap_proxy_error(&parsed.error);
    assert_eq!(
        msg,
        "[Moonshot AI] thinking is enabled but reasoning_content is missing in assistant tool call message at index 39"
    );
    assert_eq!(ty.as_deref(), Some("invalid_request_error"));
}

#[test]
fn unwrap_proxy_error_falls_back_when_no_metadata() {
    let body = r#"{"error":{"message":"Missing API key","type":"authentication_error"}}"#;
    let parsed: OpenAIErrorResponse = serde_json::from_str(body).expect("parse");
    let (msg, ty) = unwrap_proxy_error(&parsed.error);
    assert_eq!(msg, "Missing API key");
    assert_eq!(ty.as_deref(), Some("authentication_error"));
}

#[test]
fn unwrap_proxy_error_handles_non_json_raw() {
    let body = r#"{
      "error": {
        "message": "Provider returned error",
        "metadata": {
          "raw": "backend timed out",
          "provider_name": "Alibaba"
        }
      }
    }"#;
    let parsed: OpenAIErrorResponse = serde_json::from_str(body).expect("parse");
    let (msg, _) = unwrap_proxy_error(&parsed.error);
    assert!(msg.contains("[Alibaba]"), "should prefix backend name");
    assert!(
        msg.contains("backend timed out"),
        "should include raw text when it isn't JSON: got {msg:?}"
    );
}

#[test]
fn unwrap_proxy_error_metadata_present_but_no_raw_field() {
    let body = r#"{
      "error": {
        "message": "rate limited",
        "type": "rate_limit_exceeded",
        "metadata": { "provider_name": "Moonshot" }
      }
    }"#;
    let parsed: OpenAIErrorResponse = serde_json::from_str(body).expect("parse");
    let (msg, ty) = unwrap_proxy_error(&parsed.error);
    // No `raw` → return outer as-is (no prefix added).
    assert_eq!(msg, "rate limited");
    assert_eq!(ty.as_deref(), Some("rate_limit_exceeded"));
}

// ─── ProviderError::Display and is_retryable ────────────────────────

#[test]
fn api_error_display_hides_empty_error_type_brackets() {
    let err = ProviderError::ApiError {
        status: 400,
        message: "boom".to_string(),
        error_type: Some(String::new()),
    };
    let rendered = err.to_string();
    assert_eq!(rendered, "API error (400): boom");
    assert!(
        !rendered.contains("[]"),
        "Display must not print '[]' when error_type is Some(\"\")"
    );
}

#[test]
fn api_error_display_shows_non_empty_error_type() {
    let err = ProviderError::ApiError {
        status: 400,
        message: "bad".to_string(),
        error_type: Some("invalid_request_error".to_string()),
    };
    assert_eq!(
        err.to_string(),
        "API error (400) [invalid_request_error]: bad"
    );
}

#[test]
fn transient_proxy_400_retryable_on_generic_passthrough() {
    let err = ProviderError::ApiError {
        status: 400,
        message: "Provider returned error".to_string(),
        error_type: None,
    };
    assert!(
        err.is_retryable(),
        "proxy passthrough 400s must get the retry budget"
    );
}

#[test]
fn transient_proxy_400_retryable_on_empty_type_and_empty_message() {
    let err = ProviderError::ApiError {
        status: 400,
        message: String::new(),
        error_type: Some(String::new()),
    };
    assert!(err.is_retryable());
}

#[test]
fn transient_proxy_400_not_retryable_when_real_error_type_present() {
    let err = ProviderError::ApiError {
        status: 400,
        message:
            "thinking is enabled but reasoning_content is missing in assistant tool call message at index 39"
                .to_string(),
        error_type: Some("invalid_request_error".to_string()),
    };
    assert!(
        !err.is_retryable(),
        "real invalid_request_error must not be retried"
    );
}

#[test]
fn transient_proxy_400_not_retryable_on_specific_client_messages() {
    let err = ProviderError::ApiError {
        status: 400,
        message: "invalid model 'x'".to_string(),
        error_type: None,
    };
    assert!(
        !err.is_retryable(),
        "specific client-side 400 messages stay non-retryable"
    );
}

#[test]
fn is_transient_proxy_400_recognizes_known_phrases() {
    assert!(is_transient_proxy_400("Provider returned error", None));
    assert!(is_transient_proxy_400("Upstream error", Some("")));
    assert!(is_transient_proxy_400("Internal error", None));
    assert!(is_transient_proxy_400("Bad Gateway", Some("")));
    assert!(is_transient_proxy_400("Please try again", None));
    assert!(is_transient_proxy_400("", None));
}

#[test]
fn is_transient_proxy_400_rejects_actionable_messages() {
    assert!(!is_transient_proxy_400(
        "invalid api key format",
        Some("authentication_error")
    ));
    assert!(!is_transient_proxy_400(
        "model 'foo' not found",
        Some("model_not_found")
    ));
    assert!(!is_transient_proxy_400("some random reason", None));
}

// ─── needs_reasoning_content_for ────────────────────────────────────

#[test]
fn reasoning_needed_for_opencode_kimi() {
    assert!(needs_reasoning_content_for(
        "https://opencode.ai/zen/go/v1/chat/completions",
        "kimi-k2.6"
    ));
    assert!(needs_reasoning_content_for(
        "https://opencode.ai/zen/go/v1/chat/completions",
        "Kimi-K2.6"
    ));
}

#[test]
fn reasoning_needed_for_direct_moonshot() {
    assert!(needs_reasoning_content_for(
        "https://api.moonshot.ai/v1/chat/completions",
        "moonshot-v1"
    ));
}

#[test]
fn reasoning_not_needed_for_opencode_qwen() {
    assert!(!needs_reasoning_content_for(
        "https://opencode.ai/zen/go/v1/chat/completions",
        "qwen3.6-plus"
    ));
}

#[test]
fn reasoning_not_needed_for_unrelated_providers() {
    assert!(!needs_reasoning_content_for(
        "https://api.z.ai/api/coding/paas/v4/chat/completions",
        "glm-5.1"
    ));
    assert!(!needs_reasoning_content_for(
        "https://api.minimax.io/v1/chat/completions",
        "MiniMax-M2.7"
    ));
    assert!(!needs_reasoning_content_for(
        "https://api.openai.com/v1/chat/completions",
        "gpt-5"
    ));
}
