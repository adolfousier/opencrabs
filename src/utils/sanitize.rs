//! Tool input sanitization for safe display in approval messages and UI.
//!
//! When the agent calls tools like `http_request` or `bash`, the inputs may
//! contain API keys, Authorization headers, passwords, or tokens. This module
//! redacts those values before they are shown to users in Telegram/WhatsApp
//! approval dialogs or the TUI, while preserving enough context (field names,
//! non-sensitive values) for the user to understand what the tool is doing.

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{Map, Value};

/// Field name patterns (case-insensitive) whose values are always redacted.
const SENSITIVE_KEYS: &[&str] = &[
    "authorization",
    "api_key",
    "apikey",
    "api-key",
    "x-api-key",
    "x-auth-token",
    "x-access-token",
    "token",
    "secret",
    "password",
    "passwd",
    "pass",
    "credential",
    "credentials",
    "access_token",
    "refresh_token",
    "client_secret",
    "private_key",
    "auth",
    "bearer",
];

/// Regex-like patterns in bash commands to redact inline secrets.
/// Each tuple is (prefix_to_find, chars_to_keep_after_prefix).
/// We redact the rest of the token after these prefixes.
const COMMAND_SENSITIVE_PATTERNS: &[&str] = &[
    "bearer ",
    "authorization: ",
    "x-api-key: ",
    "x-auth-token: ",
    "api_key=",
    "apikey=",
    "api-key=",
    "token=",
    "secret=",
    "password=",
    "passwd=",
    "access_token=",
];

/// Returns true if a JSON object key looks like it holds a sensitive value.
fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_lowercase();
    SENSITIVE_KEYS
        .iter()
        .any(|&pat| lower == pat || lower.contains(pat))
}

/// Redact sensitive values from a bash command string.
/// Handles patterns like:
///   -H "Authorization: Bearer sk-xxx"
///   --header "X-Api-Key: abc123"
///   https://user:password@host/path
///   api_key=abc123
fn redact_command(cmd: &str) -> String {
    let mut result = cmd.to_string();

    // Redact URL passwords: https://user:PASSWORD@host → https://user:[REDACTED]@host
    // Simple approach: find ://word:word@ patterns
    if let Some(at_pos) = result.find("://") {
        let rest = &result[at_pos + 3..];
        if let Some(at_sign) = rest.find('@')
            && let Some(colon) = rest[..at_sign].find(':')
        {
            let pass_start = at_pos + 3 + colon + 1;
            let pass_end = at_pos + 3 + at_sign;
            if pass_start < pass_end && pass_end <= result.len() {
                result.replace_range(pass_start..pass_end, "[REDACTED]");
            }
        }
    }

    // Redact inline header values and query params (case-insensitive)
    let lower = result.to_lowercase();
    for pattern in COMMAND_SENSITIVE_PATTERNS {
        let mut search_start = 0;
        while let Some(pos) = lower[search_start..].find(pattern) {
            let match_pos = search_start + pos + pattern.len();
            // Find end of the secret: whitespace, quote, or end of string
            let secret_end = result[match_pos..]
                .find(['"', '\'', ' ', '&', '\n'])
                .map(|p| match_pos + p)
                .unwrap_or(result.len());
            if secret_end > match_pos {
                result.replace_range(match_pos..secret_end, "[REDACTED]");
            }
            // Advance past the pattern to avoid infinite loop
            search_start = match_pos;
            if search_start >= result.len() {
                break;
            }
        }
    }

    result
}

/// Recursively redact sensitive fields from a tool input JSON value.
///
/// - Object keys matching `SENSITIVE_KEYS` have their string values replaced
///   with `"[REDACTED]"`
/// - The `command` field (bash) has inline secret patterns redacted
/// - The `headers` object has all values for sensitive header names redacted
/// - Arrays and nested objects are recursively processed
pub fn redact_tool_input(value: &Value) -> Value {
    redact_value(value, None)
}

fn redact_value(value: &Value, parent_key: Option<&str>) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::with_capacity(map.len());
            for (k, v) in map {
                let redacted = if is_sensitive_key(k) {
                    // Redact the value regardless of type
                    Value::String("[REDACTED]".to_string())
                } else if k == "command" {
                    // Bash command: apply inline pattern redaction
                    match v.as_str() {
                        Some(cmd) => Value::String(redact_command(cmd)),
                        None => redact_value(v, Some(k)),
                    }
                } else if k == "headers" {
                    // Headers object: redact values for sensitive header names
                    redact_headers_object(v)
                } else if k == "query" || k == "params" {
                    // Query params object: redact sensitive param values
                    redact_value(v, Some(k))
                } else if k == "url" {
                    // URLs may have passwords embedded
                    match v.as_str() {
                        Some(url) => Value::String(redact_command(url)),
                        None => redact_value(v, Some(k)),
                    }
                } else {
                    redact_value(v, Some(k))
                };
                out.insert(k.clone(), redacted);
            }
            Value::Object(out)
        }
        Value::Array(arr) => {
            // If parent key is sensitive, redact the whole array
            if parent_key.map(is_sensitive_key).unwrap_or(false) {
                Value::String("[REDACTED]".to_string())
            } else {
                Value::Array(arr.iter().map(|v| redact_value(v, None)).collect())
            }
        }
        Value::String(s) => {
            if parent_key.map(is_sensitive_key).unwrap_or(false) {
                Value::String("[REDACTED]".to_string())
            } else {
                Value::String(s.clone())
            }
        }
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// Free-text secret redaction (for thinking streams, responses, intermediate text)
// ---------------------------------------------------------------------------

/// Known API key prefixes with their minimum total length.
/// Format: (prefix, min_suffix_len) — token must have at least this many chars
/// after the prefix to be considered a real key (avoids false positives on
/// short strings that happen to start with a prefix).
///
/// When adding new prefixes: prefer longer, more specific prefixes first
/// (e.g. `sk-proj-` before `sk-`). The scanner checks all prefixes, so order
/// only affects which prefix is shown in `[prefix][REDACTED]` output.
const KEY_PREFIXES: &[(&str, usize)] = &[
    // --- AI / LLM providers ---
    ("sk-proj-", 20),      // OpenAI project keys
    ("sk-ant-api03-", 20), // Anthropic API keys (full prefix)
    ("sk-ant-", 20),       // Anthropic API keys (short prefix)
    ("sk-or-v1-", 20),     // OpenRouter keys
    ("sk-cp-", 20),        // MiniMax keys
    ("sk-", 20),           // Generic OpenAI-style keys (DeepSeek, etc.)
    ("gsk_", 15),          // Groq keys
    ("nvapi-", 15),        // NVIDIA NIM keys
    ("AIzaSy", 20),        // Google AI / Firebase keys
    ("ya29.", 20),         // Google OAuth access tokens
    ("pplx-", 20),         // Perplexity keys
    ("hf_", 15),           // HuggingFace tokens
    ("r8_", 20),           // Replicate tokens
    // --- Cloud providers ---
    ("AKIA", 12), // AWS access key IDs (exactly 20 chars total)
    ("ASIA", 12), // AWS STS temporary credentials
    ("ABIA", 12), // AWS STS (another variant)
    ("ACCA", 12), // AWS CloudFront
    // --- Azure ---
    // Azure keys are 32-char base64 but headers are caught by SENSITIVE_KEYS.
    // Connection strings have a known prefix:
    ("DefaultEndpointsProtocol=", 10), // Azure Storage connection strings
    // --- Payments ---
    ("sk_live_", 15), // Stripe secret keys (live)
    ("sk_test_", 15), // Stripe secret keys (test)
    ("pk_live_", 15), // Stripe publishable keys (live)
    ("pk_test_", 15), // Stripe publishable keys (test)
    ("rk_live_", 15), // Stripe restricted keys (live)
    ("rk_test_", 15), // Stripe restricted keys (test)
    ("sq0atp-", 15),  // Square access tokens
    ("sq0csp-", 15),  // Square application secrets
    // --- Git / DevOps ---
    ("ghp_", 15),            // GitHub personal tokens
    ("gho_", 15),            // GitHub OAuth tokens
    ("ghu_", 15),            // GitHub user-to-server tokens
    ("ghs_", 15),            // GitHub server-to-server tokens
    ("github_pat_", 15),     // GitHub fine-grained tokens
    ("glpat-", 15),          // GitLab personal tokens
    ("gloas-", 15),          // GitLab OAuth tokens
    ("npm_", 15),            // npm registry tokens
    ("pypi-AgEIcHlwaS", 10), // PyPI tokens
    // --- Communication ---
    ("xoxb-", 15),    // Slack bot tokens
    ("xoxp-", 15),    // Slack user tokens
    ("xapp-", 15),    // Slack app tokens
    ("xoxs-", 15),    // Slack session tokens
    ("SG.", 20),      // SendGrid API keys
    ("xkeysib-", 15), // Brevo (Sendinblue) keys
    // --- E-commerce / SaaS ---
    ("shpat_", 15),   // Shopify admin tokens
    ("shpca_", 15),   // Shopify custom app tokens
    ("shppa_", 15),   // Shopify partner tokens
    ("shpss_", 15),   // Shopify shared secret
    ("ntn_", 15),     // Notion API tokens
    ("lin_api_", 15), // Linear API keys
    ("aio_", 15),     // Airtable tokens
    ("phc_", 15),     // PostHog keys
    // --- Infrastructure / Monitoring ---
    ("sntrys_", 15),         // Sentry auth tokens
    ("dop_v1_", 15),         // DigitalOcean tokens
    ("tskey-", 15),          // Tailscale keys
    ("tvly-", 15),           // Tavily web search keys
    ("hvs.", 15),            // HashiCorp Vault service tokens
    ("vault:v1:", 10),       // HashiCorp Vault transit-encrypted
    ("AGE-SECRET-KEY-", 10), // age encryption keys
    // --- Social / Meta ---
    ("EAA", 30),  // Facebook/Meta long-lived tokens
    ("ATTA", 30), // Trello tokens
    // --- Auth / Crypto ---
    ("eyJ", 30),    // JWT tokens (base64 of `{"`)
    ("whsec_", 15), // Webhook signing secrets
];

/// Regex for long hex strings that look like API tokens (32+ hex chars).
/// Lowered from 40 to 32 to catch Azure keys, custom service tokens, etc.
/// UUIDs are safe — they contain dashes so won't match contiguous hex.
static HEX_TOKEN_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b[0-9a-fA-F]{32,}\b").unwrap());

/// Regex for mixed alphanumeric tokens (28+ chars containing both letters and digits).
/// Catches opaque tokens like custom service keys that have no prefix.
/// Won't match pure words, pure numbers, or structured strings with separators.
static MIXED_ALNUM_TOKEN_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b[a-zA-Z0-9]{28,}\b").unwrap());

/// Redact API keys and tokens from free-form text (thinking, responses, etc.).
///
/// Catches:
/// - Known provider key prefixes (`sk-proj-...`, `xoxb-...`, `AIzaSy...`, etc.)
/// - Long hex token strings (32+ chars — Azure keys, custom tokens, etc.)
/// - Mixed alphanumeric tokens (28+ chars with both letters and digits)
/// - Bearer/Authorization inline patterns
///
/// Preserves the prefix so the user can see *what kind* of key was redacted.
pub fn redact_secrets(text: &str) -> String {
    let mut result = text.to_string();

    // 1. Redact known key prefixes — keep prefix, replace rest with [REDACTED]
    for &(prefix, min_suffix_len) in KEY_PREFIXES {
        let lower = result.to_lowercase();
        let prefix_lower = prefix.to_lowercase();
        let mut search_from = 0;
        while let Some(pos) = lower[search_from..].find(&prefix_lower) {
            let abs_pos = search_from + pos;
            let after = abs_pos + prefix.len();
            // Find end of token: next whitespace, quote, comma, backtick, or end
            let end = result[after..]
                .find(|c: char| {
                    c.is_whitespace()
                        || matches!(
                            c,
                            '"' | '\'' | ',' | '`' | ')' | ']' | '}' | '>' | '|' | ';'
                        )
                })
                .map(|p| after + p)
                .unwrap_or(result.len());
            let suffix_len = end - after;
            if suffix_len >= min_suffix_len {
                // Keep prefix visible, redact the rest
                result.replace_range(after..end, "[REDACTED]");
            }
            search_from = after + "[REDACTED]".len().min(result.len() - after);
            if search_from >= result.len() {
                break;
            }
        }
    }

    // 2. Redact long hex tokens (32+ chars — catches Azure keys, custom service tokens, etc.)
    result = HEX_TOKEN_RE
        .replace_all(&result, "[REDACTED_TOKEN]")
        .into_owned();

    // 3. Redact mixed alphanumeric tokens (28+ chars with both letters AND digits).
    //    Catches opaque keys like custom service tokens with no prefix.
    //    Only redacts if the match contains both digits and letters (avoids
    //    false positives on long words like "acknowledgement" or pure numbers).
    result = MIXED_ALNUM_TOKEN_RE
        .replace_all(&result, |caps: &regex::Captures| {
            let token = caps.get(0).unwrap().as_str();
            let has_digit = token.chars().any(|c| c.is_ascii_digit());
            let has_alpha = token.chars().any(|c| c.is_ascii_alphabetic());
            if has_digit && has_alpha {
                "[REDACTED_TOKEN]".to_string()
            } else {
                token.to_string()
            }
        })
        .into_owned();

    // 4. Redact inline "Bearer <token>" patterns
    let lower = result.to_lowercase();
    for pattern in &["bearer ", "authorization: bearer "] {
        let mut search_from = 0;
        while let Some(pos) = lower[search_from..].find(pattern) {
            let abs_pos = search_from + pos;
            let after = abs_pos + pattern.len();
            let end = result[after..]
                .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | ')'))
                .map(|p| after + p)
                .unwrap_or(result.len());
            if end > after {
                result.replace_range(after..end, "[REDACTED]");
            }
            search_from = after + "[REDACTED]".len().min(result.len() - after);
            if search_from >= result.len() {
                break;
            }
        }
    }

    result
}

/// Strip LLM-hallucinated artifacts from text before external delivery.
///
/// Removes:
/// - HTML comments (`<!-- tools-v2: ... -->`, `<!-- lens -->`, etc.)
/// - XML tool-call blocks (`<tool_call>`, `<tool_code>`, `<minimax:tool_call>`, etc.)
///
/// Use this on any text sent to Telegram, HTTP webhooks, or other external channels.
pub fn strip_llm_artifacts(text: &str) -> String {
    use crate::brain::agent::service::AgentService;

    let mut result = text.to_string();
    if result.contains("<!--") {
        result = AgentService::strip_html_comments(&result);
    }
    // Only strip XML blocks that contain valid tool-call JSON — prose mentions
    // like "we fixed the <tool_call> bug" must not be eaten.
    if AgentService::has_xml_tool_block(&result) {
        let parsed = AgentService::parse_xml_tool_calls(&result);
        if !parsed.is_empty() {
            result = AgentService::strip_xml_tool_calls(&result);
        }
    }
    result
}

/// Redact values inside a headers object for known sensitive header names.
fn redact_headers_object(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::with_capacity(map.len());
            for (k, v) in map {
                let redacted = if is_sensitive_key(k) {
                    Value::String("[REDACTED]".to_string())
                } else {
                    v.clone()
                };
                out.insert(k.clone(), redacted);
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_authorization_header() {
        let input = json!({
            "method": "POST",
            "url": "https://api.trello.com/1/cards",
            "headers": {
                "Authorization": "Bearer sk-trello-abc123",
                "Content-Type": "application/json"
            }
        });
        let out = redact_tool_input(&input);
        assert_eq!(out["headers"]["Authorization"], "[REDACTED]");
        assert_eq!(out["headers"]["Content-Type"], "application/json");
    }

    #[test]
    fn redacts_api_key_field() {
        let input = json!({"api_key": "secret123", "query": "something"});
        let out = redact_tool_input(&input);
        assert_eq!(out["api_key"], "[REDACTED]");
        assert_eq!(out["query"], "something");
    }

    #[test]
    fn redacts_bash_bearer_token() {
        let input = json!({
            "command": "curl -H \"Authorization: Bearer sk-abc123\" https://api.example.com"
        });
        let out = redact_tool_input(&input);
        let cmd = out["command"].as_str().unwrap();
        assert!(cmd.contains("[REDACTED]"), "expected REDACTED in: {cmd}");
        assert!(!cmd.contains("sk-abc123"), "secret still present: {cmd}");
    }

    #[test]
    fn redacts_url_password() {
        let input = json!({
            "url": "https://user:mysecretpass@api.example.com/v1"
        });
        let out = redact_tool_input(&input);
        let url = out["url"].as_str().unwrap();
        assert!(url.contains("[REDACTED]"), "expected REDACTED in: {url}");
        assert!(
            !url.contains("mysecretpass"),
            "password still present: {url}"
        );
    }

    #[test]
    fn preserves_non_sensitive_fields() {
        let input = json!({
            "method": "GET",
            "url": "https://api.example.com/data",
            "timeout_secs": 30
        });
        let out = redact_tool_input(&input);
        assert_eq!(out["method"], "GET");
        assert_eq!(out["timeout_secs"], 30);
    }

    // --- redact_secrets (free-text) tests ---

    #[test]
    fn redact_secrets_openai_key() {
        let text =
            "The API key is sk-proj-mrRb3y9swLqHv8ZzB9lPH0_V7RPruzdbnXJf34DxU2RCdQnhCYjS99Tj ok?";
        let out = redact_secrets(text);
        assert!(out.contains("sk-proj-[REDACTED]"), "got: {out}");
        assert!(!out.contains("mrRb3y"), "secret leaked: {out}");
        assert!(out.contains("ok?"), "trailing text lost: {out}");
    }

    #[test]
    fn redact_secrets_anthropic_key() {
        let text = "Use sk-ant-oat01-H9Uogg04aohFVZn5qymS8R for auth";
        let out = redact_secrets(text);
        assert!(out.contains("sk-ant-[REDACTED]"), "got: {out}");
        assert!(!out.contains("H9Uogg"), "secret leaked: {out}");
    }

    #[test]
    fn redact_secrets_slack_token() {
        let text = "slack token: xoxb-6222682357047-10517173104019-apMsAoYqjFSwIa4xbBqbrpUd";
        let out = redact_secrets(text);
        assert!(out.contains("xoxb-[REDACTED]"), "got: {out}");
    }

    #[test]
    fn redact_secrets_google_key() {
        let text = "key=AIzaSyFAKE_TEST_KEY_NOT_REAL_000000 for gemini";
        let out = redact_secrets(text);
        assert!(out.contains("AIzaSy[REDACTED]"), "got: {out}");
    }

    #[test]
    fn redact_secrets_hex_token() {
        let text = "auth_token=aa83802d35bb2c4471e7e96f4eaeafa6c96fe42f set";
        let out = redact_secrets(text);
        assert!(out.contains("[REDACTED_TOKEN]"), "got: {out}");
        assert!(!out.contains("aa83802d"), "secret leaked: {out}");
    }

    #[test]
    fn redact_secrets_preserves_normal_text() {
        let text = "The model is claude-3-opus and the temperature is 0.7";
        let out = redact_secrets(text);
        assert_eq!(out, text);
    }

    #[test]
    fn redact_secrets_multiple_keys() {
        let text = "OpenAI: sk-proj-AAAAAAAAAAAAAAAAAAAAAA, Groq: gsk_BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";
        let out = redact_secrets(text);
        assert!(out.contains("sk-proj-[REDACTED]"), "got: {out}");
        assert!(out.contains("gsk_[REDACTED]"), "got: {out}");
    }

    // --- New pattern tests ---

    #[test]
    fn redact_secrets_stripe_live_key() {
        let text = "stripe key: sk_live_FAKE00TEST00KEY00EXAMPLE00VAL";
        let out = redact_secrets(text);
        assert!(out.contains("sk_live_[REDACTED]"), "got: {out}");
        assert!(!out.contains("FAKE00TEST"), "secret leaked: {out}");
    }

    #[test]
    fn redact_secrets_aws_access_key() {
        let text = "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE";
        let out = redact_secrets(text);
        assert!(out.contains("AKIA[REDACTED]"), "got: {out}");
        assert!(!out.contains("IOSFODNN"), "secret leaked: {out}");
    }

    #[test]
    fn redact_secrets_sendgrid_key() {
        let text = "SENDGRID_API_KEY=SG.abc123def456ghi789jkl012mno345pqr678stu901vwx234yz";
        let out = redact_secrets(text);
        assert!(out.contains("SG.[REDACTED]"), "got: {out}");
    }

    #[test]
    fn redact_secrets_jwt_token() {
        let text = "token: eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N";
        let out = redact_secrets(text);
        assert!(out.contains("eyJ[REDACTED]"), "got: {out}");
    }

    #[test]
    fn redact_secrets_mixed_alnum_opaque_token() {
        // Simulates tokens like agentverse keys — no prefix, mixed letters+digits, 32 chars
        let text = "key: 38947394723jkhkrjkhdfiuo83489732 done";
        let out = redact_secrets(text);
        assert!(
            out.contains("[REDACTED_TOKEN]"),
            "opaque mixed-alnum token not caught: {out}"
        );
        assert!(!out.contains("38947394723"), "secret leaked: {out}");
    }

    #[test]
    fn redact_secrets_hex_32_chars() {
        // 32-char hex token (e.g. Azure API key)
        let text = "api-key: a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4 end";
        let out = redact_secrets(text);
        assert!(
            out.contains("[REDACTED_TOKEN]"),
            "32-char hex not caught: {out}"
        );
    }

    #[test]
    fn redact_secrets_preserves_short_alnum() {
        // Short alphanumeric strings should NOT be redacted
        let text = "model claude3opus version 12345 session abc123";
        let out = redact_secrets(text);
        assert_eq!(out, text, "short strings should be preserved");
    }

    #[test]
    fn redact_secrets_preserves_pure_alpha_long() {
        // Long pure-alpha strings (English words) should NOT be redacted
        let text = "the acknowledgementofresponsibility was important";
        let out = redact_secrets(text);
        assert_eq!(out, text, "pure-alpha long string should be preserved");
    }

    #[test]
    fn redact_secrets_shopify_token() {
        let text = "token: shpat_abc123def456ghi789jkl012mno";
        let out = redact_secrets(text);
        assert!(out.contains("shpat_[REDACTED]"), "got: {out}");
    }

    #[test]
    fn redact_secrets_digital_ocean_token() {
        let text = "DO_TOKEN=dop_v1_abc123def456ghi789jkl012mno345";
        let out = redact_secrets(text);
        assert!(out.contains("dop_v1_[REDACTED]"), "got: {out}");
    }
}
