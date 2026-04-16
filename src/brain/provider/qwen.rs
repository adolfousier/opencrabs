//! Qwen (DashScope) header + body-transform helpers.
//!
//! OpenCrabs talks to Qwen through the standard OpenAI-compatible DashScope
//! endpoint with a regular API key (DashScope or Alibaba Coding Plan). This
//! file only contains the wire-level fingerprinting logic that the factory
//! layers on top of `OpenAIProvider`:
//!
//! - `qwen_extra_headers()` — the four `X-DashScope-*` / `User-Agent`
//!   headers that qwen-cli emits. Matching them keeps us out of the
//!   stricter rate-limit bucket the gateway applies to unknown clients.
//! - `qwen_body_transform()` — the DashScope cache-control + metadata
//!   shape that qwen-cli posts alongside the OpenAI-compatible request.
//!
//! The older OAuth device flow, token manager, multi-account rotation, and
//! `~/.qwen/oauth_creds.json` sync were removed when Alibaba discontinued
//! Qwen OAuth — the `portal.qwen.ai` endpoint now only accepts DashScope /
//! Coding Plan API keys, and the CLI itself dropped the OAuth option.

// ── DashScope headers ─────────────────────────────────────────────────────

/// Version string sent in `User-Agent` and `X-DashScope-UserAgent`.
/// Must stay `QwenCode/<semver>` — the gateway validates the prefix.
const QWEN_CLI_VERSION: &str = "0.14.0";

/// Node-style arch token for `User-Agent` / `X-DashScope-UserAgent`.
/// qwen-cli uses Node's `process.arch` values directly.
fn node_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "arm" => "arm",
        "x86" => "ia32",
        other => other,
    }
}

/// Platform tuple baked into `User-Agent` and `X-DashScope-UserAgent`.
/// qwen-cli constructs these as `${process.platform}; ${process.arch}`
/// where `platform` is `darwin` / `linux` / `win32`.
fn user_agent_platform() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    format!("{}; {}", os, node_arch())
}

/// Extra headers sent with every DashScope request.
///
/// These are the **exact four** headers that
/// `DashScopeOpenAICompatibleProvider.buildHeaders()` emits in
/// `@qwen-code/qwen-code`:
///
/// ```text
/// User-Agent: QwenCode/<version> (<platform>; <arch>)
/// X-DashScope-CacheControl: enable
/// X-DashScope-UserAgent: QwenCode/<version> (<platform>; <arch>)
/// X-DashScope-AuthType: qwen-oauth
/// ```
///
/// The gateway fingerprints the full header set; sending any additional
/// `x-stainless-*` / SDK telemetry headers drops us into a tighter
/// rate-limit bucket, so we stick to these four.
pub fn qwen_extra_headers() -> Vec<(String, String)> {
    let ua = format!("QwenCode/{} ({})", QWEN_CLI_VERSION, user_agent_platform());
    vec![
        ("User-Agent".to_string(), ua.clone()),
        ("X-DashScope-CacheControl".to_string(), "enable".to_string()),
        ("X-DashScope-UserAgent".to_string(), ua),
        ("X-DashScope-AuthType".to_string(), "qwen-oauth".to_string()),
    ]
}

// ── DashScope body shape ──────────────────────────────────────────────────

/// Stable per-process session id, mirroring qwen-cli's `metadata.sessionId`.
/// DashScope tracks per-session quota, so reusing one id across the process
/// lifetime keeps us in a single bucket instead of looking like a fresh
/// client on every request.
fn qwen_session_id() -> &'static str {
    use std::sync::OnceLock;
    static SESSION: OnceLock<String> = OnceLock::new();
    SESSION.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

/// Per-request id, mirroring qwen-cli's `metadata.promptId`. qwen-cli uses
/// a short hex string; we use 13 hex chars derived from a random u64.
fn qwen_prompt_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut x = nanos as u64 ^ 0x9E37_79B9_7F4A_7C15;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    format!("{:013x}", x & 0x000F_FFFF_FFFF_FFFF)
}

/// Vision-capable model identifiers recognized by qwen-cli's
/// `DashScopeOpenAICompatibleProvider.isVisionModel()`: exact match on
/// `coder-model`, or prefix on `qwen-vl`, `qwen3-vl-plus`, `qwen3.5-plus`.
fn is_vision_model(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    if m == "coder-model" {
        return true;
    }
    for prefix in ["qwen-vl", "qwen3-vl-plus", "qwen3.5-plus"] {
        if m.starts_with(prefix) {
            return true;
        }
    }
    false
}

/// Normalize an OpenAI `content` field to the array form qwen-cli uses
/// when attaching cache control. Mirrors `normalizeContentToArray`.
fn normalize_content_to_array(content: &serde_json::Value) -> Vec<serde_json::Value> {
    match content {
        serde_json::Value::String(s) => {
            vec![serde_json::json!({ "type": "text", "text": s })]
        }
        serde_json::Value::Array(arr) => arr.clone(),
        _ => Vec::new(),
    }
}

/// Apply `cache_control: {type: "ephemeral"}` to the LAST part of the
/// content array. Mirrors `addCacheControlToContentArray`.
fn add_cache_control_to_content(content: &serde_json::Value) -> serde_json::Value {
    let mut arr = normalize_content_to_array(content);
    if let Some(last) = arr.last_mut()
        && let Some(obj) = last.as_object_mut()
    {
        obj.insert(
            "cache_control".to_string(),
            serde_json::json!({ "type": "ephemeral" }),
        );
    }
    serde_json::Value::Array(arr)
}

/// Rewrite a serialized OpenAI chat-completions body into the exact dialect
/// that qwen-cli's `DashScopeOpenAICompatibleProvider.buildRequest` emits.
///
/// Transforms applied:
///   1. **Cache control** — `addDashScopeCacheControl(request, stream ? "all" : "system_only")`:
///      system message (if any) gets `cache_control: {type: "ephemeral"}`
///      on its last content part; when streaming, the LAST message
///      (regardless of role) and the LAST tool also get the same tag.
///   2. **metadata** — `{sessionId, promptId}` added at the top level.
///   3. **vl_high_resolution_images: true** — added only when the model
///      is in the vision list.
///   4. No field stripping. `temperature`, `top_p`, `tool_choice`, etc.
///      pass through. DashScope's fingerprint expects these to be present
///      when the client supplies them. `max_tokens` is never synthesized.
pub fn qwen_body_transform(mut body: serde_json::Value) -> serde_json::Value {
    let obj = match body.as_object_mut() {
        Some(o) => o,
        None => return body,
    };

    let is_streaming = obj.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);

    // ── 1. Cache control on messages ────────────────────────────────────
    if let Some(serde_json::Value::Array(messages)) = obj.get_mut("messages") {
        let msg_count = messages.len();
        if msg_count > 0 {
            let system_idx = messages
                .iter()
                .position(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"));
            let last_idx = msg_count - 1;

            for (i, msg) in messages.iter_mut().enumerate() {
                let should_cache = (Some(i) == system_idx) || (is_streaming && i == last_idx);
                if !should_cache {
                    continue;
                }
                let Some(msg_obj) = msg.as_object_mut() else {
                    continue;
                };
                let content = match msg_obj.get("content") {
                    Some(c) if !c.is_null() => c.clone(),
                    _ => continue,
                };
                msg_obj.insert(
                    "content".to_string(),
                    add_cache_control_to_content(&content),
                );
            }
        }
    }

    // ── 2. Metadata ─────────────────────────────────────────────────────
    obj.insert(
        "metadata".to_string(),
        serde_json::json!({
            "sessionId": qwen_session_id(),
            "promptId": qwen_prompt_id(),
        }),
    );

    // ── 3. vl_high_resolution_images (only for vision models) ───────────
    let model = obj
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if is_vision_model(&model) {
        obj.insert(
            "vl_high_resolution_images".to_string(),
            serde_json::Value::Bool(true),
        );
    }

    // ── 4. Cache control on LAST tool (streaming only) ──────────────────
    if is_streaming
        && let Some(serde_json::Value::Array(tools)) = obj.get_mut("tools")
        && let Some(last) = tools.last_mut()
        && let Some(tool_obj) = last.as_object_mut()
    {
        tool_obj.insert(
            "cache_control".to_string(),
            serde_json::json!({ "type": "ephemeral" }),
        );
    }

    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_headers_match_qwen_cli_exactly() {
        let h = qwen_extra_headers();
        let names: Vec<&str> = h.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(h.len(), 4, "expected exactly 4 headers, got {:?}", names);
        assert!(names.contains(&"User-Agent"));
        assert!(names.contains(&"X-DashScope-CacheControl"));
        assert!(names.contains(&"X-DashScope-UserAgent"));
        assert!(names.contains(&"X-DashScope-AuthType"));
        let ua = h
            .iter()
            .find(|(k, _)| k == "User-Agent")
            .map(|(_, v)| v.clone())
            .unwrap();
        let ds_ua = h
            .iter()
            .find(|(k, _)| k == "X-DashScope-UserAgent")
            .map(|(_, v)| v.clone())
            .unwrap();
        assert_eq!(ua, ds_ua);
        assert!(ua.starts_with("QwenCode/"));
        let auth = h
            .iter()
            .find(|(k, _)| k == "X-DashScope-AuthType")
            .map(|(_, v)| v.clone())
            .unwrap();
        assert_eq!(auth, "qwen-oauth");
    }

    fn sample_body() -> serde_json::Value {
        serde_json::json!({
            "model": "coder-model",
            "messages": [
                { "role": "system", "content": "sys prompt" },
                { "role": "user", "content": "first user" },
                { "role": "assistant", "content": "asst reply" },
                { "role": "user", "content": "last user" }
            ],
            "temperature": 0.7,
            "top_p": 0.95,
            "tool_choice": "auto",
            "max_completion_tokens": 8192,
            "include_reasoning": true,
            "stream": true,
            "stream_options": { "include_usage": true },
            "tools": [
                {
                    "type": "function",
                    "function": { "name": "first_tool", "description": "", "parameters": {} }
                },
                {
                    "type": "function",
                    "function": { "name": "last_tool", "description": "", "parameters": {} }
                }
            ]
        })
    }

    #[test]
    fn body_transform_cache_control_streaming_system_and_last_message() {
        let out = qwen_body_transform(sample_body());
        let msgs = out.get("messages").and_then(|v| v.as_array()).unwrap();

        let sys = &msgs[0];
        assert_eq!(sys["role"], "system");
        assert!(sys["content"].is_array());
        assert_eq!(sys["content"][0]["type"], "text");
        assert_eq!(sys["content"][0]["cache_control"]["type"], "ephemeral");

        assert!(msgs[1]["content"].is_string());
        assert!(msgs[2]["content"].is_string());

        let u2 = &msgs[3];
        assert!(u2["content"].is_array());
        assert_eq!(u2["content"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn body_transform_non_streaming_only_tags_system() {
        let mut body = sample_body();
        body["stream"] = serde_json::json!(false);
        let out = qwen_body_transform(body);
        let msgs = out.get("messages").and_then(|v| v.as_array()).unwrap();

        assert!(msgs[0]["content"].is_array());
        assert_eq!(msgs[0]["content"][0]["cache_control"]["type"], "ephemeral");
        assert!(msgs[3]["content"].is_string());
    }

    #[test]
    fn body_transform_preserves_all_fields() {
        let out = qwen_body_transform(sample_body());
        let obj = out.as_object().unwrap();
        assert_eq!(obj.get("temperature"), Some(&serde_json::json!(0.7)));
        assert_eq!(obj.get("top_p"), Some(&serde_json::json!(0.95)));
        assert_eq!(obj.get("tool_choice"), Some(&serde_json::json!("auto")));
        assert_eq!(
            obj.get("max_completion_tokens"),
            Some(&serde_json::json!(8192))
        );
        assert_eq!(obj.get("include_reasoning"), Some(&serde_json::json!(true)));
    }

    #[test]
    fn body_transform_adds_metadata_with_session_and_prompt_ids() {
        let out = qwen_body_transform(sample_body());
        let meta = out.get("metadata").unwrap();
        assert!(meta["sessionId"].is_string());
        assert!(meta["promptId"].is_string());
    }

    #[test]
    fn body_transform_vl_flag_only_for_vision_models() {
        let out = qwen_body_transform(sample_body());
        assert_eq!(out["vl_high_resolution_images"], true);

        let mut body = sample_body();
        body["model"] = serde_json::json!("qwen3-32b");
        let out = qwen_body_transform(body);
        assert!(
            out.as_object()
                .unwrap()
                .get("vl_high_resolution_images")
                .is_none(),
            "text-only model should not carry vl_high_resolution_images"
        );
    }

    #[test]
    fn body_transform_does_not_force_max_tokens() {
        let mut body = sample_body();
        body.as_object_mut().unwrap().remove("max_tokens");
        let out = qwen_body_transform(body);
        assert!(out.as_object().unwrap().get("max_tokens").is_none());
    }

    #[test]
    fn body_transform_tags_last_tool_only_when_streaming() {
        let out = qwen_body_transform(sample_body());
        let tools = out.get("tools").and_then(|v| v.as_array()).unwrap();
        assert!(tools[0].get("cache_control").is_none());
        assert_eq!(tools[1]["cache_control"]["type"], "ephemeral");

        let mut body = sample_body();
        body["stream"] = serde_json::json!(false);
        let out = qwen_body_transform(body);
        let tools = out.get("tools").and_then(|v| v.as_array()).unwrap();
        assert!(tools[0].get("cache_control").is_none());
        assert!(tools[1].get("cache_control").is_none());
    }

    #[test]
    fn body_transform_preserves_existing_max_tokens() {
        let mut body = sample_body();
        body["max_tokens"] = serde_json::json!(4096);
        let out = qwen_body_transform(body);
        assert_eq!(out["max_tokens"], 4096);
    }

    #[test]
    fn body_transform_cache_control_on_multimodal_last_message() {
        let body = serde_json::json!({
            "model": "coder-model",
            "stream": true,
            "messages": [
                { "role": "system", "content": "sys" },
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "look at this" },
                        { "type": "image_url", "image_url": { "url": "data:..." } }
                    ]
                }
            ]
        });
        let out = qwen_body_transform(body);
        let msgs = out["messages"].as_array().unwrap();
        let u = &msgs[1];
        let content = u["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn is_vision_model_matches_qwen_cli_list() {
        assert!(is_vision_model("coder-model"));
        assert!(is_vision_model("qwen-vl-max"));
        assert!(is_vision_model("qwen-vl-max-latest"));
        assert!(is_vision_model("qwen3-vl-plus"));
        assert!(is_vision_model("qwen3.5-plus"));
        assert!(is_vision_model("CODER-MODEL"));
        assert!(!is_vision_model("qwen3-32b"));
        assert!(!is_vision_model("qwen-max"));
        assert!(!is_vision_model(""));
    }

    #[test]
    fn session_id_is_stable_within_process() {
        let a = qwen_session_id();
        let b = qwen_session_id();
        assert_eq!(a, b);
    }

    #[test]
    fn prompt_id_is_13_hex_chars() {
        let id = qwen_prompt_id();
        assert_eq!(id.len(), 13);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
