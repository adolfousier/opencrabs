#![allow(dead_code)]
//! Custom OpenAI-Compatible Provider Implementation
//!
//! Implements the Provider trait for any OpenAI-compatible API, including:
//! - Official OpenAI (GPT-4, GPT-3.5, etc.)
//! - OpenRouter (100+ models)
//! - Minimax
//! - Local LLMs via LM Studio, Ollama, LocalAI
//! - Any endpoint that speaks the OpenAI chat completions protocol

use super::error::{ProviderError, Result};
use super::rate_limiter::RateLimiter;
use super::r#trait::{Provider, ProviderStream};
use super::types::*;
use crate::brain::tokenizer::{count_message_tokens, count_tokens};
use async_trait::async_trait;
use futures::stream::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
const DEFAULT_OPENAI_API_URL: &str = "https://api.openai.com/v1/chat/completions";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// Open/close tag pairs to strip from streaming/non-streaming content.
/// Covers DeepSeek-style `<think>` and Kimi-style `<!-- reasoning -->` blocks.
/// The generic `<!--` entry catches ALL HTML comments (tools-v2, lens, /tools-v2,
/// and any future hallucinated markers) so they never reach the TUI during streaming.
/// Each entry in STRIP_CLOSE_TAGS is a list of accepted close tags (first match wins).
/// MiniMax closes `<!-- reasoning -->` with `</think>` instead of `<!-- /reasoning -->`.
/// Order matters: more specific patterns must come before the generic `<!--` catch-all.
/// NOTE: Only reasoning/markup blocks belong here — NOT XML tool-call tags.
/// Tool-call XML (`<tool_call>`, `<tool_use>`, `<result>`, etc.) must NOT be
/// stripped during streaming because the model may MENTION these tags in prose
/// (e.g. "strip `<result>` tags"). Stripping them here eats the rest of the
/// response when no closing tag arrives in the same chunk. Tool-call XML is
/// handled post-response in tool_loop.rs where the full text is available.
const STRIP_OPEN_TAGS: &[&str] = &["<think>", "<!-- reasoning -->", "<!--"];
const STRIP_CLOSE_TAGS: &[&[&str]] = &[
    &["</think>"],
    &["<!-- /reasoning -->", "</think>", "-->"], // Kimi uses <!-- /reasoning -->, MiniMax uses </think>, --> catches split-chunk close tags
    &["-->"],
];

/// Filter reasoning/markup blocks from a streaming chunk.
///
/// Tracks state across chunks via `inside_think`. Returns the portion of `text`
/// that is outside any stripped block. Handles tags split across chunk boundaries.
/// Maximum bytes to consume inside a `<think>...</think>` / `<!-- ... -->`
/// block before we emit a one-time warning that the close tag seems delayed.
///
/// The old value (400 bytes) was chosen when DeepSeek-style `<think>` blocks
/// were short. Qwen3 / Kimi / DeepSeek-R1 with `enable_thinking=true` routinely
/// emit 5-20 KB of reasoning before closing — 400 bytes blew past on every
/// turn, flipped `inside_think=false`, and leaked the rest of the reasoning
/// (including the orphan `</think>` tag) straight to the display. Raised to
/// 200 KB to fit real-world reasoning chains comfortably.
///
/// NOTE: when this cap is exceeded we no longer flip `inside_think=false`.
/// The original behaviour was designed for hallucinated open tags (e.g.
/// `<!-- tools-v2:` with no closer) — but flipping out of think mode in the
/// common long-reasoning case is strictly worse than holding until the close
/// actually arrives. We log once per block and keep suppressing.
const THINK_BLOCK_MAX_BYTES: usize = 200_000;

/// Longest open-tag-prefix we may have to hold back between chunks so
/// an open tag split across chunk boundaries still gets detected.
/// `"<!-- reasoning -->"` is the longest entry, but we only need to
/// carry up to `len(open_tag) - 1` bytes of it to bridge a split.
const MAX_OPEN_TAG_CARRY: usize = 17;

/// Returns `(display_text, reasoning_text)`.
///
/// `display_text` is the portion of the input that falls OUTSIDE every
/// stripped block — what the user should see.
///
/// `reasoning_text` is the portion that fell inside a `<think>` or
/// `<!-- reasoning -->` block. Generic HTML-comment blocks (the
/// catch-all `<!--` entry at index 2 in STRIP_OPEN_TAGS, covering
/// echoed `<!-- tools-v2: ... -->` markers etc.) are NOT treated as
/// reasoning — they go to neither output and are discarded.
///
/// Capturing reasoning lets the caller emit it as
/// `ContentDelta::ReasoningDelta` so models like Qwen that emit their
/// thinking inline inside message content (rather than via the OpenAI
/// `reasoning_content` field) still surface to the TUI as live
/// "Thinking..." content and get persisted as `details` on the final
/// assistant `DisplayMessage` — matching the UX of providers that emit
/// reasoning via the structured field natively.
fn filter_think_tags(
    text: &str,
    inside_think: &mut bool,
    active_close_tag: &mut usize,
    bytes_consumed: &mut usize,
    carry: &mut String,
) -> (String, String) {
    // Prepend any carry-over from the previous chunk so a split open
    // tag (e.g. prior chunk ended with `<!`, current chunk starts with
    // `-- tools-v2:`) can match as one unit.
    let mut owned: String;
    let input_ref: &str = if carry.is_empty() {
        text
    } else {
        owned = std::mem::take(carry);
        owned.push_str(text);
        owned.as_str()
    };
    let mut result = String::new();
    let mut reasoning = String::new();
    let mut remaining = input_ref;

    // STRIP_OPEN_TAGS indices 0 (`<think>`) and 1 (`<!-- reasoning -->`)
    // carry real reasoning content. Index 2 (`<!--` generic) catches
    // hallucinated tool markers and random HTML comments — those must
    // stay discarded.
    let is_reasoning_block = |idx: usize| idx < 2;

    loop {
        if *inside_think {
            // Safety valve: if we've consumed an unusually large amount of
            // content without finding the closing tag, log ONCE per block so
            // operators can spot a genuinely hallucinated open tag — but keep
            // `inside_think=true` and stay in suppress mode. Previously we
            // flipped out of think mode here, which leaked the rest of long
            // Qwen reasoning chains straight to the display.
            *bytes_consumed += remaining.len();
            if *bytes_consumed > THINK_BLOCK_MAX_BYTES {
                tracing::warn!(
                    "⚠️ Think-tag filter consumed {} bytes without close tag \
                     (tag_idx={}) — still waiting for close, continuing to suppress",
                    *bytes_consumed,
                    *active_close_tag,
                );
                // Reset the counter so we don't re-log every chunk; stay in
                // suppress mode until the actual close tag arrives. No
                // reasoning routing for generic `<!--` comments.
                if is_reasoning_block(*active_close_tag) {
                    reasoning.push_str(remaining);
                }
                *bytes_consumed = 0;
                break;
            }

            // Find the earliest matching close tag among the candidates for this block.
            let close_candidates = STRIP_CLOSE_TAGS[*active_close_tag];
            let earliest_close = close_candidates
                .iter()
                .filter_map(|close| remaining.find(close).map(|pos| (pos, *close)))
                .min_by_key(|(pos, _)| *pos);

            if let Some((end, close)) = earliest_close {
                if is_reasoning_block(*active_close_tag) {
                    reasoning.push_str(&remaining[..end]);
                }
                remaining = &remaining[end + close.len()..];
                *inside_think = false;
                *bytes_consumed = 0;
            } else {
                // No close in this chunk. Capture the whole remaining
                // slice as reasoning (if this IS a reasoning block) so
                // the caller can stream it live; the close tag will
                // arrive in a future chunk.
                if is_reasoning_block(*active_close_tag) {
                    reasoning.push_str(remaining);
                }
                break;
            }
        } else {
            // Find the earliest open tag
            let mut earliest: Option<(usize, usize)> = None; // (position, tag_index)
            for (i, open) in STRIP_OPEN_TAGS.iter().enumerate() {
                if let Some(pos) = remaining.find(open)
                    && earliest.is_none_or(|(best, _)| pos < best)
                {
                    earliest = Some((pos, i));
                }
            }

            if let Some((pos, tag_idx)) = earliest {
                result.push_str(&remaining[..pos]);
                remaining = &remaining[pos + STRIP_OPEN_TAGS[tag_idx].len()..];
                *inside_think = true;
                *active_close_tag = tag_idx;
                *bytes_consumed = 0;
            } else {
                // Before emitting `remaining` as final output, check if its
                // tail could be the leading prefix of an open tag that gets
                // completed by the next chunk. If so, move those bytes to
                // the carry instead of emitting them.
                let tail_keep = open_tag_prefix_len(remaining);
                if tail_keep > 0 {
                    let split_at = remaining.len() - tail_keep;
                    result.push_str(&remaining[..split_at]);
                    carry.push_str(&remaining[split_at..]);
                } else {
                    result.push_str(remaining);
                }
                break;
            }
        }
    }

    (result, reasoning)
}

/// Length of the longest suffix of `s` that's a strict prefix of any
/// marker in `markers`. Used by the streaming tool-call partitioner to
/// decide how many trailing bytes to hold back as carry when a marker
/// could be split across chunk boundaries — the exact problem local
/// llama.cpp/MLX backends create by streaming 1-3 bytes per SSE event.
///
/// Returns 0 when no suffix is a marker prefix, so the caller can emit
/// the whole working string unchanged. Respects UTF-8 char boundaries so
/// the carry always sits on a safe split point.
pub(crate) fn tool_marker_prefix_len(s: &str, markers: &[&str]) -> usize {
    let max_marker_len = markers.iter().map(|m| m.len()).max().unwrap_or(0);
    if max_marker_len <= 1 {
        return 0;
    }
    let start = s.len().saturating_sub(max_marker_len - 1);
    // Walk forward from the earliest viable start — the first match we
    // find is the longest suffix.
    for i in start..s.len() {
        if !s.is_char_boundary(i) {
            continue;
        }
        let suffix = &s[i..];
        if suffix.is_empty() {
            continue;
        }
        if markers
            .iter()
            .any(|m| m.len() > suffix.len() && m.starts_with(suffix))
        {
            return suffix.len();
        }
    }
    0
}

/// Return how many trailing bytes of `s` look like the beginning of any
/// STRIP_OPEN_TAGS entry (but not a full match). These are withheld as
/// carry so the open-tag detector can see the full tag on the next chunk.
fn open_tag_prefix_len(s: &str) -> usize {
    // Walk character boundaries from the tail so every suffix we test is
    // guaranteed to be a valid `&str`. Longest-first: return the largest
    // tail that's a proper prefix of any STRIP_OPEN_TAGS entry.
    let tail_starts = s
        .char_indices()
        .map(|(i, _)| i)
        .filter(|i| s.len() - i <= MAX_OPEN_TAG_CARRY);
    for start in tail_starts {
        let suffix = &s[start..];
        for open in STRIP_OPEN_TAGS {
            if open.len() > suffix.len() && open.starts_with(suffix) {
                return suffix.len();
            }
        }
    }
    0
}

/// Strip complete reasoning/markup blocks from non-streaming content.
fn strip_think_blocks(text: &str) -> String {
    let mut result = text.to_string();
    for (open, close_candidates) in STRIP_OPEN_TAGS.iter().zip(STRIP_CLOSE_TAGS.iter()) {
        while let Some(start) = result.find(open) {
            // Find the earliest close tag among the candidates.
            let earliest_close = close_candidates
                .iter()
                .filter_map(|close| result[start..].find(close).map(|end| (end, *close)))
                .min_by_key(|(end, _)| *end);

            if let Some((end, close)) = earliest_close {
                result = format!(
                    "{}{}",
                    &result[..start],
                    &result[start + end + close.len()..]
                );
            } else {
                // Unclosed tag — strip from open tag to end
                result = result[..start].to_string();
                break;
            }
        }
    }
    result.trim().to_string()
}

/// Extract tool_call blocks emitted as text content.
///
/// Local GGUF/MLX backends serving reasoning models will often put tool
/// calls into `message.content` instead of the structured `tool_calls`
/// field when the runtime isn't in the right template mode. Unsloth's
/// parser (`studio/backend/routes/inference.py`) solves this by scanning
/// the raw text for several formats and emitting structured tool calls
/// anyway. We handle four:
///
///   1. `<tool_call>{"name":"x","arguments":{...}}</tool_call>` — Qwen XML
///   2. `<function=x><parameter=k>v</parameter></function>` — Qwen v2 XML
///   3. `tool_call:{"id":"...","type":"function","function":{...}}` —
///      bare OpenAI-envelope prefix (no tags) that Qwen3 leaks when the
///      template isn't fully in reasoning mode but the model still knows
///      it should be calling a tool
///   4. `{"tool_calls":[{...}, ...]}` — OpenAI multi-call envelope
///      hallucinated as content text
///
/// Closing tags are treated as optional — models skip them constantly.
/// JSON parsing uses balanced-brace counting with a string/escape guard
/// so `{` or `}` inside argument strings doesn't confuse the extractor.
///
/// Returns `(tool_calls, cleaned_text)` where `cleaned_text` is the input
/// with every matched block removed. If nothing matches, the text is
/// returned unchanged — safe to call on any content.
pub(crate) fn extract_text_tool_calls(text: &str) -> (Vec<(String, serde_json::Value)>, String) {
    // Cheap pre-check so non-matching content pays nothing.
    let has_claude_style = KNOWN_TOOL_NAMES
        .iter()
        .any(|t| text.contains(&format!("<{}>", t)));
    if !text.contains("<tool_call>")
        && !text.contains("<function=")
        && !text.contains("tool_call:")
        && !text.contains("\"tool_calls\"")
        && !text.contains("\"tool_call\"")
        && !has_claude_style
    {
        return (Vec::new(), text.to_string());
    }

    // Collect byte ranges of every matched block; strip them at the end.
    let mut tool_calls: Vec<(String, serde_json::Value)> = Vec::new();
    let mut strip_ranges: Vec<(usize, usize)> = Vec::new();

    // Pass 1 — Claude-style `<TOOLNAME><PARAM>val</PARAM></TOOLNAME>`.
    // Seen in logs 2026-04-17 14:27 where unsloth Qwen emitted a
    // <bash><command>curl ...</command></bash> block instead of calling
    // the structured `tool_calls` API. Run BEFORE the tag-based scan so
    // `<bash>...</bash>` doesn't get mistaken for a prose <bash> mention
    // elsewhere.
    if has_claude_style {
        for (start, end, name, input) in extract_claude_style_tool_calls(text) {
            tool_calls.push((name, input));
            strip_ranges.push((start, end));
        }
    }

    let mut i: usize = 0;
    let bytes = text.as_bytes();

    while i < bytes.len() {
        let tc_at = text[i..].find("<tool_call>").map(|r| i + r);
        let fn_at = text[i..].find("<function=").map(|r| i + r);
        let bare_at = text[i..].find("tool_call:").map(|r| i + r);
        let arr_at = text[i..].find("\"tool_calls\"").map(|r| i + r);
        // Singular `{"tool_call": {...}}` envelope. Logs 2026-04-17 03:07
        // confirmed Qwen3 emits this shape (often with the JSON colons
        // missing between key and value — "tool_call" { ... }) when the
        // template isn't fully in reasoning mode.
        let sing_at = {
            let candidate = text[i..].find("\"tool_call\"").map(|r| i + r);
            // Skip matches that are actually the plural `"tool_calls"` —
            // those are handled by arr_at with different extraction logic.
            match candidate {
                Some(p)
                    if text.as_bytes().get(p + "\"tool_call\"".len() - 1).copied()
                        != Some(b'"') =>
                {
                    None
                }
                Some(p) if text[p..].starts_with("\"tool_calls\"") => None,
                other => other,
            }
        };
        let next = [tc_at, fn_at, bare_at, arr_at, sing_at]
            .into_iter()
            .flatten()
            .min();
        let Some(start) = next else { break };

        if bare_at == Some(start) {
            // Guard: "tool_call:" must be a bare marker, not a substring inside a
            // larger identifier like "set_tool_call:true" or "my_tool_call:fn".
            // Require the preceding byte (if any) to be a word boundary.
            if start > 0 {
                let prev = text.as_bytes()[start - 1];
                let is_boundary = prev.is_ascii_whitespace()
                    || matches!(
                        prev,
                        b',' | b';' | b':' | b'[' | b'(' | b'{' | b'\n' | b'\r'
                    );
                if !is_boundary {
                    i = start + "tool_call:".len();
                    continue;
                }
            }
            let body_start = start + "tool_call:".len();
            let brace_rel = text[body_start..]
                .char_indices()
                .find(|(_, c)| !c.is_whitespace())
                .map(|(idx, _)| idx);
            let brace_abs = match brace_rel {
                Some(rel) if text.as_bytes().get(body_start + rel) == Some(&b'{') => {
                    body_start + rel
                }
                _ => {
                    // Not a JSON envelope — advance past the marker.
                    i = body_start;
                    continue;
                }
            };
            match extract_balanced_json(&text[brace_abs..]) {
                Some(consumed) => {
                    let json_slice = &text[brace_abs..brace_abs + consumed];
                    if let Some(call) = parse_qwen_tool_json(json_slice) {
                        tool_calls.push(call);
                        strip_ranges.push((start, brace_abs + consumed));
                        i = brace_abs + consumed;
                        continue;
                    }
                    // Unrecognized JSON shape — skip the bare marker but not
                    // the JSON (it may still be prose with legitimate JSON).
                    i = body_start;
                }
                None => {
                    i = body_start;
                }
            }
            continue;
        } else if arr_at == Some(start) {
            // `{"tool_calls":[...]}` envelope hallucinated as content text.
            // Find the wrapping `{`; it should be within a short window of
            // the `"tool_calls"` key (typically `{"tool_calls":` or
            // `{ "tool_calls":`). If it's further away, this is probably
            // prose like `the \"tool_calls\" field` — bail.
            let wrapper = text[..start].rfind('{');
            let wrapper_start = match wrapper {
                Some(br) if start - br <= 4 => br,
                _ => {
                    i = start + "\"tool_calls\"".len();
                    continue;
                }
            };
            match extract_balanced_json(&text[wrapper_start..]) {
                Some(consumed) => {
                    let env_slice = &text[wrapper_start..wrapper_start + consumed];
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(env_slice)
                        && let Some(arr) = v.get("tool_calls").and_then(|a| a.as_array())
                    {
                        let mut found_any = false;
                        for item in arr {
                            if let Some(call) = parse_tool_call_value(item) {
                                tool_calls.push(call);
                                found_any = true;
                            }
                        }
                        if found_any {
                            strip_ranges.push((wrapper_start, wrapper_start + consumed));
                            i = wrapper_start + consumed;
                            continue;
                        }
                    }
                    i = start + "\"tool_calls\"".len();
                }
                None => {
                    i = start + "\"tool_calls\"".len();
                }
            }
            continue;
        } else if sing_at == Some(start) {
            // `{"tool_call": {...}}` singular envelope. Wrapper `{` within
            // ~4 bytes of the key. Parse tolerantly — the model sometimes
            // drops the colon between key and value (`"tool_call" {`) so
            // strict serde_json fails. Fall back to recovering name /
            // arguments from the inner object via regex.
            let wrapper = text[..start].rfind('{');
            let wrapper_start = match wrapper {
                Some(br) if start - br <= 4 => br,
                _ => {
                    i = start + "\"tool_call\"".len();
                    continue;
                }
            };
            match extract_balanced_json_tolerant(&text[wrapper_start..]) {
                Some(consumed) => {
                    let env_slice = &text[wrapper_start..wrapper_start + consumed];
                    let recovered = recover_tool_call_from_malformed_json(env_slice);
                    if let Some(call) = recovered {
                        tool_calls.push(call);
                        strip_ranges.push((wrapper_start, wrapper_start + consumed));
                        i = wrapper_start + consumed;
                        continue;
                    }
                    i = start + "\"tool_call\"".len();
                }
                None => {
                    i = start + "\"tool_call\"".len();
                }
            }
            continue;
        } else if tc_at == Some(start) {
            // <tool_call>{json}</tool_call>? — closing tag optional
            let body_start = start + "<tool_call>".len();
            // Skip whitespace to the opening brace.
            let brace_rel = text[body_start..]
                .char_indices()
                .find(|(_, c)| !c.is_whitespace())
                .map(|(idx, _)| idx);
            let brace_abs = match brace_rel {
                Some(rel) if text.as_bytes().get(body_start + rel) == Some(&b'{') => {
                    body_start + rel
                }
                _ => {
                    // Not a JSON tool_call — advance past the tag and keep scanning.
                    i = body_start;
                    continue;
                }
            };
            match extract_balanced_json(&text[brace_abs..]) {
                Some(consumed) => {
                    let json_slice = &text[brace_abs..brace_abs + consumed];
                    if let Some(call) = parse_qwen_tool_json(json_slice) {
                        tool_calls.push(call);
                    }
                    // Include the optional `</tool_call>` (and any whitespace
                    // between `}` and it) in the strip range.
                    let mut end = brace_abs + consumed;
                    let after = &text[end..];
                    let ws_len = after.len() - after.trim_start().len();
                    if after.trim_start().starts_with("</tool_call>") {
                        end += ws_len + "</tool_call>".len();
                    }
                    strip_ranges.push((start, end));
                    i = end;
                }
                None => {
                    // Unbalanced JSON — likely a prose mention like
                    // "strip the <tool_call> tag". Don't strip; move on.
                    i = body_start;
                }
            }
        } else {
            // <function=name>...</function>? — parameters optional
            let tag_start = start;
            // Find the `>` closing the opening tag.
            let after = &text[tag_start..];
            let open_end = match after.find('>') {
                Some(r) => tag_start + r + 1,
                None => {
                    i = tag_start + "<function=".len();
                    continue;
                }
            };
            let name = text[tag_start + "<function=".len()..open_end - 1].trim();
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                i = open_end;
                continue;
            }
            // Body ends at first of: </tool_call>, next <function=, or </function>.
            // Unsloth avoids </function> as boundary because code params can contain
            // it literally — but we keep it as a last resort since our param set
            // doesn't embed Python code that closes the tag name.
            let tail = &text[open_end..];
            let candidates = [
                tail.find("</tool_call>").map(|r| (r, "</tool_call>".len())),
                tail.find("<function=").map(|r| (r, 0usize)),
                tail.find("</function>").map(|r| (r, "</function>".len())),
            ];
            let pick = candidates.iter().filter_map(|o| *o).min_by_key(|(r, _)| *r);
            let (body_rel, close_len) = match pick {
                Some(p) => p,
                None => {
                    // No boundary — take until end of string (open-ended).
                    (tail.len(), 0)
                }
            };
            let body = &tail[..body_rel];
            let input = parse_function_params(body);
            tool_calls.push((name.to_string(), input));
            let end = open_end + body_rel + close_len;
            strip_ranges.push((start, end));
            i = end;
        }
    }

    if strip_ranges.is_empty() {
        return (tool_calls, text.to_string());
    }

    // Rebuild text with blocks removed, keeping everything outside them.
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for (s, e) in strip_ranges {
        if s > cursor {
            out.push_str(&text[cursor..s]);
        }
        cursor = e;
    }
    if cursor < text.len() {
        out.push_str(&text[cursor..]);
    }
    (tool_calls, out.trim().to_string())
}

/// Tool names we recognise when the model emits Claude-style native XML
/// invocations — `<TOOLNAME><PARAM>value</PARAM></TOOLNAME>`. Keeping the
/// set explicit rather than matching any `<\w+>` pair avoids false
/// positives on prose that mentions tags (e.g. `<html>`, `<body>`,
/// `<script>`).
const KNOWN_TOOL_NAMES: &[&str] = &[
    "bash",
    "ls",
    "glob",
    "grep",
    "read_file",
    "write_file",
    "edit_file",
    "patch_file",
    "web_search",
    "web_fetch",
    "web_request",
    "http_request",
    "plan",
    "task_manager",
    "cron_manage",
    "memory_search",
    "session_search",
    "lsp",
    "agent",
    "slack_send",
    "telegram_send",
    "discord_send",
    "trello_send",
];

/// Extract `<TOOLNAME><PARAM>value</PARAM>…</TOOLNAME>` invocations. The
/// outer tag name must be one of `KNOWN_TOOL_NAMES`; the body must
/// contain at least one `<param>value</param>` pair with matching
/// open/close tag. Returns `(start, end, name, args)` byte ranges so
/// the caller can add them to `strip_ranges`.
fn extract_claude_style_tool_calls(
    text: &str,
) -> Vec<(usize, usize, String, serde_json::Value)> {
    let mut results = Vec::new();
    let mut cursor = 0;

    while cursor < text.len() {
        // Find the next `<TOOLNAME>` from our allowlist.
        let mut best: Option<(usize, &'static str)> = None;
        for &tool in KNOWN_TOOL_NAMES {
            let needle_owned = format!("<{}>", tool);
            if let Some(rel) = text[cursor..].find(&needle_owned) {
                let abs = cursor + rel;
                if best.is_none_or(|(b, _)| abs < b) {
                    best = Some((abs, tool));
                }
            }
        }
        let Some((start, tool_name)) = best else { break };
        let open_tag_len = tool_name.len() + 2; // `<` + name + `>`
        let body_start = start + open_tag_len;

        let close_tag = format!("</{}>", tool_name);
        let Some(close_rel) = text[body_start..].find(&close_tag) else {
            // No close — advance past the open and keep scanning.
            cursor = body_start;
            continue;
        };
        let close_abs = body_start + close_rel;
        let body = &text[body_start..close_abs];

        let params = parse_xml_param_pairs(body);
        if params.is_empty() {
            cursor = close_abs + close_tag.len();
            continue;
        }

        let mut map = serde_json::Map::new();
        for (k, v) in params {
            map.insert(k, serde_json::Value::String(v));
        }
        let end = close_abs + close_tag.len();
        results.push((start, end, tool_name.to_string(), serde_json::Value::Object(map)));
        cursor = end;
    }
    results
}

/// Extract `<name>value</name>` pairs from a Claude-style tool body.
/// Only accepts alphanumeric+underscore tag names so arbitrary nested
/// XML doesn't accidentally register as a parameter.
fn parse_xml_param_pairs(body: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut cursor = 0;
    while cursor < body.len() {
        // Find next `<` followed by an identifier.
        let Some(lt_rel) = body[cursor..].find('<') else { break };
        let lt_abs = cursor + lt_rel;
        let after_lt = &body[lt_abs + 1..];
        // Identifier must be lowercase letters/digits/underscore.
        let name_len = after_lt
            .bytes()
            .take_while(|&b| b.is_ascii_alphanumeric() || b == b'_')
            .count();
        if name_len == 0 || after_lt.as_bytes().get(name_len) != Some(&b'>') {
            cursor = lt_abs + 1;
            continue;
        }
        let name = &after_lt[..name_len];
        let body_start = lt_abs + 1 + name_len + 1; // past `>`
        let close = format!("</{}>", name);
        let Some(close_rel) = body[body_start..].find(&close) else {
            break;
        };
        let value = body[body_start..body_start + close_rel].trim().to_string();
        pairs.push((name.to_string(), value));
        cursor = body_start + close_rel + close.len();
    }
    pairs
}

/// Consume a balanced JSON object starting at `s[0] == '{'`. Returns the byte
/// length through the matching closing `}`, or `None` if unbalanced. Tracks
/// string + escape state so braces inside argument strings don't confuse the
/// depth counter.
pub(crate) fn extract_balanced_json(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'{') {
        return None;
    }
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;
    for (idx, &b) in bytes.iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if in_string {
            match b {
                b'\\' => escape = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse the JSON inside a `<tool_call>{...}</tool_call>` block or a bare
/// `tool_call:{...}` envelope. Accepts every shape we've seen in the wild:
/// Qwen's canonical `{"name":"x","arguments":{...}}`, MiniMax's
/// `tool_name`/`args`, the OpenAI envelope
/// `{"id":"...","type":"function","function":{"name":"...","arguments":{...}}}`,
/// and stringified arguments (`"arguments": "{\"k\":1}"`).
fn parse_qwen_tool_json(json: &str) -> Option<(String, serde_json::Value)> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    parse_tool_call_value(&v)
}

/// Value-level parser shared by the JSON-string path and the
/// `{"tool_calls":[{...}, ...]}` multi-envelope path.
fn parse_tool_call_value(v: &serde_json::Value) -> Option<(String, serde_json::Value)> {
    // Name can live at the top level OR nested under `function` (OpenAI shape).
    let name = v
        .get("name")
        .and_then(|n| n.as_str())
        .or_else(|| v.get("tool_name").and_then(|n| n.as_str()))
        .or_else(|| {
            v.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
        })
        // Fallback: `function` itself might be a string (legacy OpenAI
        // function-call format: `{"function": "bash", ...}`).
        .or_else(|| {
            v.get("function")
                .and_then(|f| if f.is_string() { f.as_str() } else { None })
        })?
        .to_string();
    if name.is_empty() {
        return None;
    }
    // Arguments follow the same pattern — may be nested under `function`
    // for the OpenAI envelope shape.
    let args_val = v
        .get("arguments")
        .or_else(|| v.get("args"))
        .or_else(|| v.get("input"))
        .or_else(|| v.get("parameters"))
        .or_else(|| v.get("function").and_then(|f| f.get("arguments")))
        .or_else(|| v.get("function").and_then(|f| f.get("parameters")));
    let input = match args_val {
        Some(serde_json::Value::String(s)) => {
            serde_json::from_str(s).unwrap_or(serde_json::json!({}))
        }
        Some(other) => other.clone(),
        None => serde_json::json!({}),
    };
    Some((name, input))
}

/// Balanced-brace walker that tolerates tokens where the model dropped
/// structural characters (colons between key and value). Consumes as much
/// as needed to find the matching closing `}`. Unlike `extract_balanced_json`
/// this one is called only when we know the content is a malformed envelope
/// emitted by a local model — the caller then runs `recover_tool_call_from_malformed_json`
/// to extract name + arguments via regex rather than serde_json.
fn extract_balanced_json_tolerant(s: &str) -> Option<usize> {
    // Behaviour is identical to the strict version — the tolerance lives in
    // the subsequent parser, not in brace matching.
    extract_balanced_json(s)
}

/// Recover a tool call from a `{"tool_call": {"name": "...", "arguments": {...}}}`
/// envelope even when the model mangled the JSON. Seen in the wild:
///
///   `{"tool_call" {"name" "bash", "arguments" {"command" "..."}}}`
///
/// (note the missing colons after the keys). Strict serde_json refuses these,
/// so we walk via regex: find `"name"` followed by a string literal, find
/// `"command"` / `"arguments"` similarly, and stitch the result into a clean
/// `(name, input)` tuple.
fn recover_tool_call_from_malformed_json(env: &str) -> Option<(String, serde_json::Value)> {
    // First try strict parsing — cheap path for well-formed envelopes.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(env) {
        let inner = v
            .get("tool_call")
            .or_else(|| v.get("function"))
            .cloned()
            .unwrap_or(v);
        if let Some(call) = parse_tool_call_value(&inner) {
            return Some(call);
        }
    }

    // Fallback: regex extract name + common arg fields. Keep the regex
    // simple — we only handle the primitive value types local models
    // commonly emit (strings, numbers, booleans). Nested object arguments
    // need the strict path above.
    use regex::Regex;
    use std::sync::LazyLock;
    static NAME_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#""name"\s*:?\s*"([^"]+)""#).unwrap());
    // Capture every `"key" <optional-colon> <value>` pair inside the
    // arguments block. Values can be strings, numbers, or booleans.
    static ARG_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#""([a-zA-Z_][a-zA-Z0-9_]*)"\s*:?\s*("([^"\\]|\\.)*"|true|false|-?\d+(\.\d+)?)"#,
        )
        .unwrap()
    });
    let name_cap = NAME_RE.captures(env)?;
    let name = name_cap.get(1)?.as_str().to_string();
    if name.is_empty() {
        return None;
    }
    // Skip the `"name"` match itself when collecting args.
    let name_end = name_cap.get(0)?.end();
    let args_region = &env[name_end..];
    let mut map = serde_json::Map::new();
    for cap in ARG_RE.captures_iter(args_region) {
        let key = cap.get(1).map(|m| m.as_str().to_string());
        let raw = cap.get(2).map(|m| m.as_str().to_string());
        if let (Some(k), Some(r)) = (key, raw) {
            // Reserved keys we don't want to treat as arguments.
            if matches!(
                k.as_str(),
                "name" | "tool_call" | "type" | "id" | "function"
            ) {
                continue;
            }
            let val = if let Some(stripped) = r.strip_prefix('"').and_then(|s| s.strip_suffix('"'))
            {
                serde_json::Value::String(stripped.replace("\\\"", "\""))
            } else if r == "true" {
                serde_json::Value::Bool(true)
            } else if r == "false" {
                serde_json::Value::Bool(false)
            } else if let Ok(n) = r.parse::<i64>() {
                serde_json::Value::Number(n.into())
            } else if let Ok(f) = r.parse::<f64>()
                && let Some(n) = serde_json::Number::from_f64(f)
            {
                serde_json::Value::Number(n)
            } else {
                continue;
            };
            map.insert(k, val);
        }
    }
    Some((name, serde_json::Value::Object(map)))
}

/// Parse `<parameter=key>value</parameter>` pairs out of a function body.
/// When the body contains none, returns an empty object — the caller may
/// still have a valid tool call that just takes no arguments.
fn parse_function_params(body: &str) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    let mut i = 0usize;
    while let Some(rel) = body[i..].find("<parameter=") {
        let tag_start = i + rel;
        let after = &body[tag_start..];
        let Some(gt) = after.find('>') else { break };
        let key = body[tag_start + "<parameter=".len()..tag_start + gt].trim();
        if key.is_empty() {
            i = tag_start + gt + 1;
            continue;
        }
        let val_start = tag_start + gt + 1;
        let tail = &body[val_start..];
        // Value ends at next `<parameter=` or `</parameter>` (whichever comes first).
        let end_at_param = tail.find("</parameter>");
        let end_at_next = tail.find("<parameter=");
        let end = match (end_at_param, end_at_next) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let (val, next_i) = match end {
            Some(rel) => {
                // If we ended on </parameter>, skip past it; if on the next
                // opening tag, leave cursor on it.
                let skip = if end_at_param == Some(rel) {
                    rel + "</parameter>".len()
                } else {
                    rel
                };
                (tail[..rel].trim().to_string(), val_start + skip)
            }
            None => (tail.trim().to_string(), body.len()),
        };
        map.insert(key.to_string(), serde_json::Value::String(val));
        i = next_i;
    }
    serde_json::Value::Object(map)
}

/// Dynamic token provider — called on every request to get the current bearer token.
/// Used by Copilot provider where the token rotates every ~30 minutes.
pub type TokenFn = Arc<dyn Fn() -> String + Send + Sync>;

/// Optional request-body transform hook — called just before each outgoing
/// chat-completions request is serialized. Lets a provider mutate the JSON
/// body to match a vendor-specific dialect (e.g. qwen-cli's content-array
/// shape + metadata fields) without polluting the generic OpenAI path.
pub type BodyTransformFn = Arc<dyn Fn(serde_json::Value) -> serde_json::Value + Send + Sync>;

/// Optional dynamic base-URL provider. When set, `send_url()` will call
/// this on every request instead of using the stored `base_url`. Used by
/// qwen where `resource_url` can change mid-session after a token refresh.
pub type BaseUrlFn = Arc<dyn Fn() -> String + Send + Sync>;

/// Optional async auth-refresh hook called after a 401/403 response. The
/// hook is expected to refresh the bearer token (so the next `token_fn()`
/// call returns a new one) and return `Ok(())` on success. The provider
/// then retries the failed request exactly once.
pub type AuthRefreshFn = Arc<
    dyn Fn() -> std::pin::Pin<
            Box<dyn std::future::Future<Output = std::result::Result<(), String>> + Send>,
        > + Send
        + Sync,
>;

pub type AuthInvalidateFn = Arc<dyn Fn() + Send + Sync>;

/// OpenAI provider for GPT models
#[derive(Clone)]
pub struct OpenAIProvider {
    api_key: String,
    base_url: String,
    client: Client,
    custom_default_model: Option<String>,
    name: String,
    /// When set, swap to this model for requests containing images.
    vision_model: Option<String>,
    /// Extra headers injected into every request (e.g. GitHub Copilot API versioning).
    pub(crate) extra_headers: Vec<(String, String)>,
    /// Configured context window size (overrides model-name heuristics).
    configured_context_window: Option<u32>,
    /// Optional dynamic token provider (overrides api_key when set).
    token_fn: Option<TokenFn>,
    /// Proactive rate limiter — shared via Arc so all clones throttle together.
    /// Used for OpenRouter `:free` models (~3s between requests).
    rate_limiter: Option<Arc<RateLimiter>>,
    /// Optional body-transform hook applied to the serialized request body
    /// right before it is sent. Used by providers (e.g. qwen) that need a
    /// vendor-specific dialect on top of the standard OpenAI shape.
    body_transform: Option<BodyTransformFn>,
    /// Optional dynamic base-URL provider. When set, each outgoing request
    /// uses the value returned by this callback instead of `base_url`.
    base_url_fn: Option<BaseUrlFn>,
    /// Optional async auth-refresh hook. When set, a 401/403 response
    /// triggers a single retry after calling this hook.
    auth_refresh_fn: Option<AuthRefreshFn>,
    /// Optional auth-invalidate hook. Called when the refresh hook succeeds
    /// but the retried request STILL gets 401/403 — meaning the refreshed
    /// token is also dead. The callback clears credentials so re-auth is
    /// triggered on next startup.
    auth_invalidate_fn: Option<AuthInvalidateFn>,
    /// When set, overrides the automatic retry config selection in
    /// `retry_config()`. Used by `RotatingQwenProvider` to disable
    /// retry-on-rate-limit for sub-providers (rotation handles 429).
    retry_config_override: Option<super::retry::RetryConfig>,
}

impl OpenAIProvider {
    /// Returns true if this provider targets OpenRouter (detected by base_url).
    fn is_openrouter(&self) -> bool {
        self.base_url.to_lowercase().contains("openrouter")
    }

    /// Pick a retry config tuned for this (provider, model) pair.
    ///
    /// - Qwen OAuth matches qwen-cli's DEFAULT_RETRY_OPTIONS (retry 429s
    ///   in-place, Retry-After honored) because its shared upstream window
    ///   closes briefly after 2–3 tool calls then reopens within seconds —
    ///   falling back on the first 429 drops the session into the fallback
    ///   chain unnecessarily.
    /// - OpenRouter `:free` models get the same treatment: the 20 req/min
    ///   window is shared across the key and reopens quickly, and the
    ///   proactive rate_limiter already paces requests to ~15/min, so any
    ///   429 that does leak through is almost certainly a transient window
    ///   flip rather than a true quota exhaustion. Retrying in-place keeps
    ///   the user on the free tier instead of silently burning paid credits
    ///   on the fallback chain.
    /// - All other providers keep the default (bail-to-fallback on 429).
    fn retry_config(&self, model: &str) -> super::retry::RetryConfig {
        if let Some(ref ovr) = self.retry_config_override {
            return ovr.clone();
        }
        // Qwen, OpenRouter, and :free models: retry on 429 with backoff.
        // OpenRouter upstream providers often have tight per-minute windows
        // that reopen within seconds — bailing to fallback on the first 429
        // is wasteful when a 3-retry backoff would get through.
        if self.name == "qwen" || self.is_openrouter() || model.ends_with(":free") {
            super::retry::RetryConfig::qwen_cli_match()
        } else {
            super::retry::RetryConfig::default()
        }
    }

    /// Create a new OpenAI provider with official API
    pub fn new(api_key: String) -> Self {
        let client = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
            .pool_idle_timeout(DEFAULT_POOL_IDLE_TIMEOUT)
            .pool_max_idle_per_host(2)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            api_key,
            base_url: DEFAULT_OPENAI_API_URL.to_string(),
            client,
            custom_default_model: None,
            name: "openai".to_string(),
            vision_model: None,
            extra_headers: vec![],
            configured_context_window: None,
            token_fn: None,
            rate_limiter: None,
            body_transform: None,
            base_url_fn: None,
            auth_refresh_fn: None,
            auth_invalidate_fn: None,
            retry_config_override: None,
        }
    }

    /// Create provider for local LLM (LM Studio, Ollama, etc.)
    pub fn local(base_url: String) -> Self {
        let client = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
            .pool_idle_timeout(DEFAULT_POOL_IDLE_TIMEOUT)
            .pool_max_idle_per_host(2)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            api_key: "not-needed".to_string(),
            base_url,
            client,
            custom_default_model: None,
            name: "openai-compatible".to_string(),
            vision_model: None,
            extra_headers: vec![],
            configured_context_window: None,
            token_fn: None,
            rate_limiter: None,
            body_transform: None,
            base_url_fn: None,
            auth_refresh_fn: None,
            auth_invalidate_fn: None,
            retry_config_override: None,
        }
    }

    /// Create with custom base URL
    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        let client = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
            .pool_idle_timeout(DEFAULT_POOL_IDLE_TIMEOUT)
            .pool_max_idle_per_host(2)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            api_key,
            base_url,
            client,
            custom_default_model: None,
            name: "openai-compatible".to_string(),
            vision_model: None,
            extra_headers: vec![],
            configured_context_window: None,
            token_fn: None,
            rate_limiter: None,
            body_transform: None,
            base_url_fn: None,
            auth_refresh_fn: None,
            auth_invalidate_fn: None,
            retry_config_override: None,
        }
    }

    /// Add extra headers to every request (e.g. API versioning).
    pub fn with_extra_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.extra_headers = headers;
        self
    }

    /// Set a configured context window size that overrides model-name heuristics.
    pub fn with_context_window(mut self, size: u32) -> Self {
        self.configured_context_window = Some(size);
        self
    }

    /// Set provider name (for logging)
    pub fn with_name(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self
    }

    /// Set custom default model (useful for local LLMs with specific model names)
    pub fn with_default_model(mut self, model: String) -> Self {
        self.custom_default_model = Some(model);
        self
    }

    /// Set a dynamic token provider (overrides static api_key in headers).
    /// Used by Copilot where the bearer token rotates every ~30 minutes.
    pub fn with_token_fn(mut self, f: TokenFn) -> Self {
        self.token_fn = Some(f);
        self
    }

    /// Set vision model — used by the `analyze_image` tool as a provider-native
    /// vision backend when Gemini vision isn't configured.
    pub fn with_vision_model(mut self, model: String) -> Self {
        self.vision_model = Some(model);
        self
    }

    /// Set a proactive rate limiter — enforces minimum interval between API
    /// calls to stay under provider rate limits (e.g. OpenRouter :free at 20/min).
    /// Takes an `Arc<RateLimiter>` so multiple provider instances share ONE limiter.
    pub fn with_rate_limiter(mut self, limiter: Arc<RateLimiter>) -> Self {
        self.rate_limiter = Some(limiter);
        self
    }

    /// Install a body-transform hook. The hook receives the fully-serialized
    /// JSON body just before it is sent and returns a (possibly modified)
    /// JSON value that will be serialized in its place. Used by providers
    /// that need a vendor-specific dialect on top of the OpenAI shape.
    pub fn with_body_transform(mut self, f: BodyTransformFn) -> Self {
        self.body_transform = Some(f);
        self
    }

    /// Install a dynamic base-URL provider. When set, every outgoing request
    /// resolves its URL via this callback instead of the stored `base_url`.
    /// Used by qwen where `resource_url` can change after a token refresh.
    pub fn with_base_url_fn(mut self, f: BaseUrlFn) -> Self {
        self.base_url_fn = Some(f);
        self
    }

    /// Install an async auth-refresh hook. On a 401/403 response the
    /// provider will call this once, wait for it to resolve, and then
    /// retry the failed request exactly once with the refreshed token.
    pub fn with_auth_refresh_fn(mut self, f: AuthRefreshFn) -> Self {
        self.auth_refresh_fn = Some(f);
        self
    }

    /// Install an auth-invalidate hook. Called when a refreshed token is
    /// *still* rejected (401/403 after retry), meaning the OAuth credentials
    /// are dead and must be cleared so re-auth is triggered.
    pub fn with_auth_invalidate_fn(mut self, f: AuthInvalidateFn) -> Self {
        self.auth_invalidate_fn = Some(f);
        self
    }

    /// Override the automatic retry config selection. Used inside
    /// `RotatingQwenProvider` to disable retry-on-rate-limit so 429s
    /// rotate immediately instead of burning ~45s in backoff per account.
    pub fn with_retry_config(mut self, config: super::retry::RetryConfig) -> Self {
        self.retry_config_override = Some(config);
        self
    }

    /// Serialize a request body to JSON, applying the optional body_transform
    /// hook. Returns a `serde_json::Value` ready to pass to `.json(&value)`.
    fn encode_body<T: Serialize>(&self, body: &T) -> Result<serde_json::Value> {
        let mut v = serde_json::to_value(body)?;
        if let Some(ref f) = self.body_transform {
            v = f(v);
        }
        Ok(v)
    }

    /// Resolve the URL to POST this request to. Uses `base_url_fn` when set
    /// (for providers whose endpoint can change mid-session), otherwise
    /// returns the stored `base_url`.
    fn send_url(&self) -> String {
        if let Some(ref f) = self.base_url_fn {
            let u = f();
            if !u.is_empty() {
                return u;
            }
        }
        self.base_url.clone()
    }

    /// Returns true if a ProviderError represents an auth failure that
    /// should trigger an auth-refresh retry (401/403).
    fn is_auth_error(err: &ProviderError) -> bool {
        matches!(
            err,
            ProviderError::InvalidApiKey
                | ProviderError::ApiError {
                    status: 401 | 403,
                    ..
                }
        )
    }

    /// Get the configured vision model name (if any).
    pub fn vision_model(&self) -> Option<&str> {
        self.vision_model.as_deref()
    }

    /// Build request headers
    fn headers(&self) -> std::result::Result<reqwest::header::HeaderMap, ProviderError> {
        let mut headers = reqwest::header::HeaderMap::new();

        // Resolve the bearer token: dynamic token_fn takes priority over static api_key
        let bearer_key = if let Some(ref f) = self.token_fn {
            let token = f();
            if token.is_empty() { None } else { Some(token) }
        } else if self.api_key != "not-needed" {
            Some(self.api_key.trim().to_string())
        } else {
            None
        };

        if let Some(key) = bearer_key {
            let header_value: reqwest::header::HeaderValue =
                format!("Bearer {}", key).parse().map_err(|_| {
                    tracing::error!(
                        "API key contains invalid characters (length={}). Check keys.toml.",
                        key.len()
                    );
                    ProviderError::InvalidApiKey
                })?;
            headers.insert(reqwest::header::AUTHORIZATION, header_value);
        }

        headers.insert(
            reqwest::header::CONTENT_TYPE,
            "application/json".parse().expect("valid content-type"),
        );
        // Explicit Accept — matches what the `openai` npm SDK sends. Without
        // this, reqwest falls back to `accept: */*` which is a visible
        // fingerprint difference from qwen-cli / most SDK clients and at
        // least one gateway (portal.qwen.ai DashScope) rate-limits on it.
        headers.insert(
            reqwest::header::ACCEPT,
            "application/json".parse().expect("valid accept"),
        );

        // OpenRouter-specific optimization headers
        if self.base_url.to_lowercase().contains("openrouter") {
            if let (Ok(k1), Ok(v1)) = (
                "HTTP-Referer".parse::<reqwest::header::HeaderName>(),
                "https://opencrabs.com".parse::<reqwest::header::HeaderValue>(),
            ) {
                headers.insert(k1, v1);
            }
            if let (Ok(k2), Ok(v2)) = (
                "X-Title".parse::<reqwest::header::HeaderName>(),
                "OpenCrabs".parse::<reqwest::header::HeaderValue>(),
            ) {
                headers.insert(k2, v2);
            }
            if let (Ok(k3), Ok(v3)) = (
                "X-OpenRouter-Category".parse::<reqwest::header::HeaderName>(),
                "cli-agent,personal-agent,programming-app".parse::<reqwest::header::HeaderValue>(),
            ) {
                headers.insert(k3, v3);
            }
            tracing::debug!("OpenRouter optimization headers attached");
        }

        for (key, value) in &self.extra_headers {
            if let (Ok(k), Ok(v)) = (
                key.parse::<reqwest::header::HeaderName>(),
                value.parse::<reqwest::header::HeaderValue>(),
            ) {
                headers.insert(k, v);
            }
        }

        Ok(headers)
    }

    /// Convert our generic request to OpenAI-specific format
    fn to_openai_request(&self, request: LLMRequest) -> OpenAIRequest {
        let mut messages = Vec::new();

        // Debug: log system brain
        if let Some(ref system) = request.system {
            tracing::debug!("System brain present: {} chars", system.len());
        } else {
            tracing::warn!("NO SYSTEM BRAIN in request!");
        }

        // Add system message if present
        if let Some(system) = request.system {
            messages.push(OpenAIMessage {
                role: "system".to_string(),
                content: Some(serde_json::Value::String(system)),
                tool_calls: None,
                tool_call_id: None,
            });
        }

        // Add conversation messages
        for msg in request.messages {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "system",
            };

            // Separate content blocks by type
            let mut text_parts = Vec::new();
            let mut image_parts: Vec<serde_json::Value> = Vec::new();
            let mut tool_uses = Vec::new();
            let mut tool_results = Vec::new();

            for block in msg.content {
                match block {
                    ContentBlock::Text { text } => {
                        text_parts.push(text);
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        tool_uses.push((id, name, input));
                    }
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    } => {
                        tool_results.push((tool_use_id, content));
                    }
                    ContentBlock::Thinking { .. } => {
                        // OpenAI-compatible providers don't support thinking blocks; skip.
                    }
                    ContentBlock::Image { source } => {
                        let url = match source {
                            ImageSource::Base64 { media_type, data } => {
                                format!("data:{};base64,{}", media_type, data)
                            }
                            ImageSource::Url { url } => url,
                        };
                        image_parts.push(serde_json::json!({
                            "type": "image_url",
                            "image_url": { "url": url }
                        }));
                    }
                }
            }

            // Build content value: array when images present, string otherwise
            let make_content =
                |texts: &[String], images: &[serde_json::Value]| -> Option<serde_json::Value> {
                    if !images.is_empty() {
                        let mut parts: Vec<serde_json::Value> = Vec::new();
                        if !texts.is_empty() {
                            parts.push(serde_json::json!({
                                "type": "text",
                                "text": texts.join("\n")
                            }));
                        }
                        parts.extend(images.iter().cloned());
                        Some(serde_json::Value::Array(parts))
                    } else if !texts.is_empty() {
                        Some(serde_json::Value::String(texts.join("\n")))
                    } else {
                        None
                    }
                };

            // Handle assistant messages with tool calls
            if !tool_uses.is_empty() {
                let openai_tool_calls = tool_uses
                    .into_iter()
                    .map(|(id, name, input)| OpenAIToolCall {
                        id,
                        r#type: "function".to_string(),
                        function: OpenAIFunctionCall {
                            name,
                            arguments: serde_json::to_string(&input).unwrap_or_default(),
                        },
                    })
                    .collect();

                let content_val = make_content(&text_parts, &image_parts);

                messages.push(OpenAIMessage {
                    role: role.to_string(),
                    content: content_val,
                    tool_calls: Some(openai_tool_calls),
                    tool_call_id: None,
                });
            }
            // Handle tool result messages
            else if !tool_results.is_empty() {
                for (tool_use_id, content) in tool_results {
                    messages.push(OpenAIMessage {
                        role: "tool".to_string(),
                        content: Some(serde_json::Value::String(content)),
                        tool_calls: None,
                        tool_call_id: Some(tool_use_id),
                    });
                }
            }
            // Handle regular text messages (with optional images)
            else {
                let content_val = make_content(&text_parts, &image_parts)
                    .unwrap_or(serde_json::Value::String(String::new()));

                messages.push(OpenAIMessage {
                    role: role.to_string(),
                    content: Some(content_val),
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
        }

        // Convert tools to OpenAI format
        let tools: Option<Vec<OpenAITool>> = request.tools.map(|tools| {
            tools
                .iter()
                .map(|tool| OpenAITool {
                    r#type: "function".to_string(),
                    function: OpenAIFunction {
                        name: tool.name.clone(),
                        description: tool.description.clone(),
                        parameters: tool.input_schema.clone(),
                    },
                })
                .collect()
        });

        // Newer OpenAI models (gpt-4.1-*, gpt-5-*, o1-*, o3-*) require
        // max_completion_tokens instead of max_tokens. Use the new field
        // for these models and fall back to max_tokens for everything else.
        let uses_completion_tokens = uses_max_completion_tokens(&request.model);
        let (max_tokens, max_completion_tokens) = if uses_completion_tokens {
            (None, request.max_tokens)
        } else {
            (request.max_tokens, None)
        };

        // Set tool_choice to "auto" when tools are present so the model
        // knows it is allowed to call them (MiniMax requires this explicitly).
        let tool_choice = tools
            .as_ref()
            .filter(|t| !t.is_empty())
            .map(|_| serde_json::json!("auto"));

        // Enable reasoning/thinking for OpenRouter and compatible endpoints.
        // Detection is intentionally broad — models that don't support the field ignore it.
        let base = self.base_url.to_lowercase();
        let include_reasoning = if base.contains("openrouter")
            || base.contains("openrouter.ai")
            || std::env::var("OPENCRABS_ENABLE_REASONING").is_ok()
        {
            Some(true)
        } else {
            None
        };

        OpenAIRequest {
            model: request.model,
            messages,
            temperature: request.temperature,
            max_tokens,
            max_completion_tokens,
            stream: Some(request.stream),
            stream_options: None,
            tools,
            tool_choice,
            include_reasoning,
        }
    }

    /// Convert to Anthropic-format request for OpenRouter with prompt caching.
    /// OpenRouter accepts this format and passes cache_control through to Anthropic
    /// models, caching the system prompt and tools across turns.
    fn to_anthropic_or_request(&self, request: LLMRequest) -> AnthropicORRequest {
        let cache = AnthropicORCacheControl {
            r#type: "ephemeral".to_string(),
        };

        // System prompt as cached content blocks
        let system = request.system.map(|s| {
            vec![AnthropicORSystemBlock {
                r#type: "text".to_string(),
                text: s,
                cache_control: Some(cache.clone()),
            }]
        });

        // Messages with content blocks
        let messages: Vec<AnthropicORMessage> = request
            .messages
            .into_iter()
            .map(|msg| {
                let role = match msg.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::System => "user", // system → user for Anthropic
                };

                let content: Vec<AnthropicORContentBlock> = msg
                    .content
                    .into_iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text { text } => Some(AnthropicORContentBlock::Text {
                            text,
                            cache_control: None,
                        }),
                        ContentBlock::ToolUse { id, name, input } => {
                            Some(AnthropicORContentBlock::ToolUse { id, name, input })
                        }
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => Some(AnthropicORContentBlock::ToolResult {
                            tool_use_id,
                            content,
                        }),
                        ContentBlock::Thinking { .. } | ContentBlock::Image { .. } => None,
                    })
                    .collect();

                AnthropicORMessage {
                    role: role.to_string(),
                    content,
                }
            })
            .collect();

        // Tools with cache_control on the last one
        let tools = request.tools.map(|tools| {
            let len = tools.len();
            tools
                .into_iter()
                .enumerate()
                .map(|(i, t)| AnthropicORTool {
                    name: t.name,
                    description: t.description,
                    input_schema: t.input_schema,
                    cache_control: if i == len - 1 {
                        Some(cache.clone())
                    } else {
                        None
                    },
                })
                .collect()
        });

        AnthropicORRequest {
            model: request.model,
            messages,
            system,
            max_tokens: request.max_tokens.unwrap_or(16384),
            temperature: request.temperature,
            tools,
            stream: Some(request.stream),
        }
    }

    /// Convert OpenAI response to our generic format
    #[allow(clippy::wrong_self_convention)]
    fn from_openai_response(&self, response: OpenAIResponse) -> LLMResponse {
        let choice = response
            .choices
            .into_iter()
            .next()
            .unwrap_or_else(|| OpenAIChoice {
                index: 0,
                message: OpenAIMessage {
                    role: "assistant".to_string(),
                    content: Some(serde_json::Value::String(String::new())),
                    tool_calls: None,
                    tool_call_id: None,
                },
                finish_reason: Some("error".to_string()),
            });

        // Convert content to content blocks
        let mut content_blocks = Vec::new();

        // Add text content if present, stripping <think>...</think> reasoning blocks
        if let Some(ref content_val) = choice.message.content {
            let text = match content_val {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Array(parts) => {
                    // Extract text from content parts array
                    parts
                        .iter()
                        .filter_map(|p| {
                            if p.get("type")?.as_str()? == "text" {
                                p.get("text")?.as_str().map(String::from)
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                }
                _ => String::new(),
            };
            if !text.is_empty() {
                let clean = strip_think_blocks(&text);
                if !clean.is_empty() {
                    content_blocks.push(ContentBlock::Text { text: clean });
                }
            }
        }

        // Fallback: local GGUF/MLX backends (llama.cpp, MLX, LM Studio, Ollama)
        // commonly emit Qwen tool calls as `<tool_call>{...}</tool_call>` text
        // inside `content` instead of the structured `tool_calls` field — this
        // happens whenever the runtime isn't launched with the right jinja +
        // reasoning template. Scan the extracted Text blocks and promote any
        // tool-call tags to ContentBlock::ToolUse so they actually execute.
        let structured_tool_calls_present = choice
            .message
            .tool_calls
            .as_ref()
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        if !structured_tool_calls_present {
            let mut recovered: Vec<ContentBlock> = Vec::new();
            for block in content_blocks.iter_mut() {
                if let ContentBlock::Text { text } = block {
                    let (calls, cleaned) = extract_text_tool_calls(text);
                    if calls.is_empty() {
                        continue;
                    }
                    tracing::info!(
                        "Recovered {} tool call(s) from text content (local-model fallback)",
                        calls.len()
                    );
                    *text = cleaned;
                    for (idx, (name, input)) in calls.into_iter().enumerate() {
                        recovered.push(ContentBlock::ToolUse {
                            id: format!("call_text_{}", idx),
                            name,
                            input,
                        });
                    }
                }
            }
            // Drop any Text blocks we just emptied, then append the recovered
            // tool-use blocks in the order they appeared.
            content_blocks.retain(|b| match b {
                ContentBlock::Text { text } => !text.trim().is_empty(),
                _ => true,
            });
            content_blocks.extend(recovered);
        }

        // Convert tool_calls to ToolUse content blocks
        if let Some(tool_calls) = choice.message.tool_calls {
            tracing::debug!(
                "Converting {} tool calls from OpenAI response",
                tool_calls.len()
            );
            for tool_call in tool_calls {
                // Parse arguments JSON string
                let input =
                    serde_json::from_str(&tool_call.function.arguments).unwrap_or_else(|e| {
                        tracing::warn!(
                            "Failed to parse tool arguments for {}: {}",
                            tool_call.function.name,
                            e
                        );
                        serde_json::json!({})
                    });

                tracing::debug!(
                    "Converted tool call: {} with id {}",
                    tool_call.function.name,
                    tool_call.id
                );

                content_blocks.push(ContentBlock::ToolUse {
                    id: tool_call.id,
                    name: tool_call.function.name,
                    input,
                });
            }
        }

        // Detect models that dump tool JSON as text instead of structured calls
        let has_tool_text = content_blocks.iter().any(|b| {
            if let ContentBlock::Text { text } = b {
                (text.contains("\"function\"") && text.contains("\"arguments\""))
                    || (text.contains("tool_call") && text.contains("\"name\""))
                    || (text.contains("```json") && text.contains("\"command\""))
            } else {
                false
            }
        });
        let has_structured_tools = content_blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolUse { .. }));
        if has_tool_text && !has_structured_tools {
            tracing::warn!(
                "Model returned tool call JSON as text — likely does not support function calling"
            );
            content_blocks.push(ContentBlock::Text {
                text: "\n\n⚠️ **This model does not support function calling.** Tool requests were returned as text instead of executable calls. Consider switching to a model that supports tool use (e.g. Claude, GPT-4, Gemini).".to_string(),
            });
        }

        // Map finish_reason to StopReason
        let stop_reason = choice
            .finish_reason
            .and_then(|reason| match reason.as_str() {
                "stop" => Some(StopReason::EndTurn),
                "length" => Some(StopReason::MaxTokens),
                "tool_calls" | "function_call" => Some(StopReason::ToolUse),
                _ => None,
            });

        LLMResponse {
            id: response.id,
            model: response.model,
            content: content_blocks,
            stop_reason,
            usage: TokenUsage {
                input_tokens: response.usage.prompt_tokens.unwrap_or(0),
                output_tokens: response.usage.completion_tokens.unwrap_or(0),
                cache_creation_tokens: response.usage.cache_creation_input_tokens.unwrap_or(0),
                cache_read_tokens: response.usage.effective_cache_read(),
                ..Default::default()
            },
        }
    }

    /// Handle API error response
    async fn handle_error(&self, response: reqwest::Response) -> ProviderError {
        let status = response.status().as_u16();

        // Extract Retry-After header for rate limits
        let retry_after = response.headers().get("retry-after").and_then(|v| {
            v.to_str().ok().and_then(|s| {
                // Retry-After can be either seconds or HTTP date
                // Try parsing as seconds first
                s.parse::<u64>().ok()
            })
        });

        // Try to parse error body
        if let Ok(error_body) = response.json::<OpenAIErrorResponse>().await {
            let message = if status == 429 {
                // Enhance rate limit error message
                if let Some(secs) = retry_after {
                    format!(
                        "{} (retry after {} seconds)",
                        error_body.error.message, secs
                    )
                } else {
                    format!(
                        "{} (rate limited, please retry later)",
                        error_body.error.message
                    )
                }
            } else {
                error_body.error.message
            };

            return if status == 429 {
                tracing::warn!("[RATE_LIMIT] {} → {}: {}", self.name, status, message,);
                ProviderError::RateLimitExceeded(message)
            } else {
                ProviderError::ApiError {
                    status,
                    message,
                    error_type: Some(error_body.error.error_type.unwrap_or_default()),
                }
            };
        }

        // Fallback error
        if status == 429 {
            let message = if let Some(secs) = retry_after {
                format!("Rate limit exceeded (retry after {} seconds)", secs)
            } else {
                "Rate limit exceeded, please retry later".to_string()
            };
            ProviderError::RateLimitExceeded(message)
        } else {
            ProviderError::ApiError {
                status,
                message: "Unknown error".to_string(),
                error_type: None,
            }
        }
    }

    /// Execute an Anthropic-format request (used for OpenRouter prompt caching).
    /// OpenRouter returns OpenAI-format responses even when sent Anthropic format.
    async fn complete_with_anthropic_format(
        &self,
        model: String,
        message_count: usize,
        anthropic_request: AnthropicORRequest,
        retry_config: super::retry::RetryConfig,
    ) -> Result<LLMResponse> {
        use super::retry::retry_with_backoff;

        let tool_count = anthropic_request
            .tools
            .as_ref()
            .map(|t| t.len())
            .unwrap_or(0);
        tracing::info!(
            "OpenRouter (Anthropic format): model={}, messages={}, tools={}, cache_control=system+last_tool",
            model,
            message_count,
            tool_count
        );

        // Proactive pacing
        if let Some(ref limiter) = self.rate_limiter {
            let waited = limiter.wait().await;
            if !waited.is_zero() {
                tracing::debug!("Rate limiter: waited {:?} before request", waited);
            }
        }

        let result = retry_with_backoff(
            || async {
                let body = self.encode_body(&anthropic_request)?;
                let response = self
                    .client
                    .post(self.send_url())
                    .headers(self.headers()?)
                    .json(&body)
                    .send()
                    .await?;

                let status = response.status();
                if !status.is_success() {
                    return Err(self.handle_error(response).await);
                }

                let openai_response: OpenAIResponse = response.json().await?;
                let llm_response = self.from_openai_response(openai_response);

                // Log cache tokens if present
                if llm_response.usage.cache_creation_tokens > 0
                    || llm_response.usage.cache_read_tokens > 0
                {
                    tracing::info!(
                        "Cache: creation={}, read={}, total_cached={}",
                        llm_response.usage.cache_creation_tokens,
                        llm_response.usage.cache_read_tokens,
                        llm_response.usage.cache_creation_tokens
                            + llm_response.usage.cache_read_tokens
                    );
                }

                Ok(llm_response)
            },
            &retry_config,
        )
        .await;

        // Handle 400 token field mismatch — retry with swapped fields
        if let Err(ref e) = result
            && let ProviderError::ApiError {
                status: 400,
                message,
                ..
            } = e
            && is_token_field_mismatch(message)
        {
            tracing::warn!("Retrying with swapped max_tokens/max_completion_tokens");
            return Box::pin(self.complete_with_anthropic_format(
                model,
                message_count,
                anthropic_request,
                retry_config,
            ))
            .await;
        }

        result
    }

    /// Execute an Anthropic-format streaming request to OpenRouter.
    /// OpenRouter returns OpenAI-format SSE responses even when sent Anthropic format.
    async fn stream_with_anthropic_format(
        &self,
        model: String,
        message_count: usize,
        anthropic_request: AnthropicORRequest,
    ) -> Result<ProviderStream> {
        use super::retry::retry_with_backoff;

        let tool_count = anthropic_request
            .tools
            .as_ref()
            .map(|t| t.len())
            .unwrap_or(0);
        tracing::info!(
            "OpenRouter stream (Anthropic format): model={}, messages={}, tools={}, cache_control=system+last_tool",
            model,
            message_count,
            tool_count
        );

        // Proactive pacing
        if let Some(ref limiter) = self.rate_limiter {
            let waited = limiter.wait().await;
            if !waited.is_zero() {
                tracing::debug!("Rate limiter: waited {:?} before streaming request", waited);
            }
        }

        let response = retry_with_backoff(
            || async {
                let body = self.encode_body(&anthropic_request)?;
                let response = self
                    .client
                    .post(self.send_url())
                    .headers(self.headers()?)
                    .json(&body)
                    .send()
                    .await?;

                let status = response.status();
                if !status.is_success() {
                    return Err(self.handle_error(response).await);
                }

                Ok(response)
            },
            &self.retry_config(&model),
        )
        .await?;

        // Parse the SSE stream — OpenRouter returns OpenAI-format SSE.
        // total_input_tokens=0 since we don't have tiktoken counts for Anthropic format.
        self.parse_openai_stream(response, 0)
    }

    /// Parse an OpenAI-compatible SSE stream into a ProviderStream.
    /// `total_input_tokens` is used as fallback usage on stream end if no real usage arrives.
    fn parse_openai_stream(
        &self,
        response: reqwest::Response,
        total_input_tokens: usize,
    ) -> Result<ProviderStream> {
        use futures::stream::StreamExt;

        let byte_stream = response.bytes_stream();
        let buffer = std::sync::Arc::new(std::sync::Mutex::new(String::new()));

        // Accumulated state for a single streamed tool call
        #[derive(Debug, Clone, Default)]
        struct ToolCallAccum {
            id: String,
            name: String,
            arguments: String,
        }

        /// State persisted across SSE chunks via Arc<Mutex<_>>
        struct StreamState {
            emitted_message_start: bool,
            emitted_content_start: bool,
            emitted_content_stop: bool,
            seen_delta_content: bool,
            tool_calls: std::collections::HashMap<usize, ToolCallAccum>,
            pending_stop_reason: Option<crate::brain::provider::types::StopReason>,
        }

        let state = std::sync::Arc::new(std::sync::Mutex::new(StreamState {
            emitted_message_start: false,
            emitted_content_start: false,
            emitted_content_stop: false,
            seen_delta_content: false,
            tool_calls: std::collections::HashMap::new(),
            pending_stop_reason: None,
        }));

        let event_stream = byte_stream
            .map(move |chunk_result| -> Vec<std::result::Result<StreamEvent, ProviderError>> {
                match chunk_result {
                    Err(e) => vec![Err(ProviderError::StreamError(e.to_string()))],
                    Ok(chunk) => {
                        let raw_text = String::from_utf8_lossy(&chunk);
                        tracing::debug!(
                            "[OR_STREAM_RAW] chunk ({} bytes): {}",
                            raw_text.len(),
                            raw_text.chars().take(500).collect::<String>()
                        );
                        let mut buf = buffer.lock().expect("SSE buffer lock poisoned");
                        buf.push_str(&raw_text);

                        let mut events = Vec::new();
                        let mut st = state.lock().expect("SSE state lock");

                        while let Some(newline_pos) = buf.find('\n') {
                            let line = buf[..newline_pos].trim().to_string();
                            buf.drain(..=newline_pos);

                            if let Some(json_str) = line.strip_prefix("data: ") {
                                if json_str == "[DONE]" {
                                    if st.emitted_content_start && !st.emitted_content_stop {
                                        events.push(Ok(StreamEvent::ContentBlockStop { index: 0 }));
                                        st.emitted_content_stop = true;
                                    }
                                    for (_idx, accum) in st.tool_calls.drain() {
                                        let input = serde_json::from_str(&accum.arguments)
                                            .unwrap_or_else(|_| serde_json::json!({}));
                                        let tool_index = _idx + 1;
                                        events.push(Ok(StreamEvent::ContentBlockStart {
                                            index: tool_index,
                                            content_block: ContentBlock::ToolUse {
                                                id: accum.id,
                                                name: accum.name,
                                                input,
                                            },
                                        }));
                                        events.push(Ok(StreamEvent::ContentBlockStop { index: tool_index }));
                                    }
                                    if let Some(stop_reason) = st.pending_stop_reason.take() {
                                        events.push(Ok(StreamEvent::MessageDelta {
                                            delta: crate::brain::provider::types::MessageDelta {
                                                stop_reason: Some(stop_reason),
                                                stop_sequence: None,
                                            },
                                            usage: crate::brain::provider::types::TokenUsage {
                                                input_tokens: total_input_tokens as u32,
                                                output_tokens: 0, ..Default::default() },
                                        }));
                                    }
                                    events.push(Ok(StreamEvent::MessageStop));
                                    continue;
                                }

                                match serde_json::from_str::<OpenAIStreamChunk>(json_str) {
                                    Ok(chunk) => {
                                        if !st.emitted_message_start && !chunk.id.is_empty() {
                                            st.emitted_message_start = true;
                                            let model = chunk.model.clone().unwrap_or_default();
                                            events.push(Ok(StreamEvent::MessageStart {
                                                message: crate::brain::provider::types::StreamMessage {
                                                    id: chunk.id,
                                                    model,
                                                    role: Role::Assistant,
                                                    usage: crate::brain::provider::types::TokenUsage {
                                                        input_tokens: 0,
                                                        output_tokens: 0, ..Default::default() },
                                                },
                                            }));
                                        }

                                        let delta_content = chunk.choices.first()
                                            .and_then(|c| c.delta.as_ref())
                                            .and_then(|d| d.content.as_ref());

                                        let finish_reason_str = chunk.choices.first()
                                            .and_then(|c| c.finish_reason.as_ref());

                                        // Emit content BEFORE handling finish — some providers
                                        // (OpenRouter Elephant, short responses) send content and
                                        // finish_reason in the same chunk. The old code did
                                        // `continue` on finish, silently dropping that content.
                                        if let Some(text) = delta_content {
                                            if !st.emitted_content_start {
                                                st.emitted_content_start = true;
                                                st.seen_delta_content = true;
                                                events.push(Ok(StreamEvent::ContentBlockStart {
                                                    index: 0,
                                                    content_block: ContentBlock::Text { text: text.clone() },
                                                }));
                                            } else {
                                                events.push(Ok(StreamEvent::ContentBlockDelta {
                                                    index: 0,
                                                    delta: ContentDelta::TextDelta { text: text.clone() },
                                                }));
                                            }
                                        }

                                        if finish_reason_str.is_some() {
                                            if st.emitted_content_start && !st.emitted_content_stop {
                                                events.push(Ok(StreamEvent::ContentBlockStop { index: 0 }));
                                                st.emitted_content_stop = true;
                                            }
                                            // Convert finish_reason to StopReason for downstream
                                            let stop_reason = match finish_reason_str.map(|s| s.as_str()) {
                                                Some("stop") => Some(crate::brain::provider::types::StopReason::EndTurn),
                                                Some("tool_calls") | Some("function_call") => Some(crate::brain::provider::types::StopReason::ToolUse),
                                                Some("length") => Some(crate::brain::provider::types::StopReason::MaxTokens),
                                                Some("content_filter") => Some(crate::brain::provider::types::StopReason::EndTurn),
                                                _ => Some(crate::brain::provider::types::StopReason::EndTurn),
                                            };
                                            st.pending_stop_reason = stop_reason;
                                            if let Some(usage) = chunk.usage.as_ref() {
                                                let input = usage.prompt_tokens.unwrap_or(0);
                                                let output = usage.completion_tokens.unwrap_or(0);
                                                let mut token_usage = crate::brain::provider::types::TokenUsage {
                                                    input_tokens: input,
                                                    output_tokens: output,
                                                    ..Default::default()
                                                };
                                                if let Some(cache_create) = usage.cache_creation_input_tokens {
                                                    token_usage.cache_creation_tokens = cache_create;
                                                }
                                                let cache_read = usage.effective_cache_read();
                                                if cache_read > 0 {
                                                    token_usage.cache_read_tokens = cache_read;
                                                }
                                                events.push(Ok(StreamEvent::MessageDelta {
                                                    delta: crate::brain::provider::types::MessageDelta {
                                                        stop_reason: None,
                                                        stop_sequence: None,
                                                    },
                                                    usage: token_usage,
                                                }));
                                            }
                                            continue;
                                        }
                                    }
                                    Err(e) => {
                                        tracing::debug!("[STREAM_PARSE] Failed to parse SSE chunk: {}", e);
                                    }
                                }
                            }
                        }

                        // ── Non-streaming fallback ──────────────────
                        // Some OpenRouter upstreams don't support streaming
                        // and return a plain JSON blob. Delegate to the
                        // dedicated nonstream_compat module.
                        if events.is_empty()
                            && !st.emitted_message_start
                            && super::nonstream_compat::is_nonstream_response(&buf)
                            && let Some(synth) = super::nonstream_compat::synthesize_stream_events(&buf)
                        {
                            st.emitted_message_start = true;
                            st.emitted_content_start = true;
                            st.emitted_content_stop = true;
                            buf.clear();
                            events.extend(synth);
                        }

                        if events.is_empty() {
                            vec![]
                        } else {
                            events
                        }
                    }
                }
            })
            .filter(|events| {
                let non_empty = !events.is_empty();
                async move { non_empty }
            })
            .flat_map(futures::stream::iter);

        Ok(Box::pin(event_stream))
    }
}

#[async_trait]
impl Provider for OpenAIProvider {
    async fn complete(&self, request: LLMRequest) -> Result<LLMResponse> {
        use super::retry::retry_with_backoff;

        let model = request.model.clone();
        let message_count = request.messages.len();
        let retry_config = self.retry_config(&model);

        let mut openai_request = self.to_openai_request(request);

        let tool_count = openai_request.tools.as_ref().map(|t| t.len()).unwrap_or(0);
        tracing::info!(
            "OpenAI API request: model={}, messages={}, max_tokens={:?}, max_completion_tokens={:?}, tools={}",
            model,
            message_count,
            openai_request.max_tokens,
            openai_request.max_completion_tokens,
            tool_count
        );
        if tool_count == 0 {
            tracing::warn!(
                "OpenAI request has NO tools - LLM won't know about file/bash operations!"
            );
        }

        // Proactive pacing: stay under provider rate limits (e.g. OpenRouter :free at 20/min)
        if let Some(ref limiter) = self.rate_limiter {
            let waited = limiter.wait().await;
            if !waited.is_zero() {
                tracing::debug!(
                    "Rate limiter: waited {:?} before request to {}",
                    waited,
                    self.base_url
                );
            }
        }

        // Retry the entire API call with exponential backoff
        let result = retry_with_backoff(
            || async {
                tracing::debug!("Sending request to OpenAI API: {}", self.base_url);
                let body = self.encode_body(&openai_request)?;
                let response = self
                    .client
                    .post(self.send_url())
                    .headers(self.headers()?)
                    .json(&body)
                    .send()
                    .await?;

                let status = response.status();
                tracing::debug!("OpenAI API response status: {}", status);

                if !status.is_success() {
                    return Err(self.handle_error(response).await);
                }

                let openai_response: OpenAIResponse = response.json().await?;
                let llm_response = self.from_openai_response(openai_response);

                tracing::info!(
                    "OpenAI API response: input_tokens={}, output_tokens={}, stop_reason={:?}",
                    llm_response.usage.input_tokens,
                    llm_response.usage.output_tokens,
                    llm_response.stop_reason
                );

                Ok(llm_response)
            },
            &retry_config,
        )
        .await;

        // Resilient fallback: if the API rejected max_tokens / max_completion_tokens,
        // swap the fields and retry once.
        if let Err(ref e) = result {
            if is_token_field_mismatch(&e.to_string()) {
                tracing::warn!(
                    "Token field mismatch for model {}, retrying with swapped fields",
                    model
                );
                openai_request.swap_token_fields();
                return retry_with_backoff(
                    || async {
                        let body = self.encode_body(&openai_request)?;
                        let response = self
                            .client
                            .post(self.send_url())
                            .headers(self.headers()?)
                            .json(&body)
                            .send()
                            .await?;
                        if !response.status().is_success() {
                            return Err(self.handle_error(response).await);
                        }
                        let openai_response: OpenAIResponse = response.json().await?;
                        Ok(self.from_openai_response(openai_response))
                    },
                    &retry_config,
                )
                .await;
            }

            // Auth-refresh fallback: on 401/403, call the refresh hook once
            // (if installed) and retry the request a single time with the
            // rotated token. Used by qwen where OAuth tokens expire mid-session.
            if Self::is_auth_error(e)
                && let Some(ref refresh) = self.auth_refresh_fn
            {
                tracing::warn!("{} auth error — refreshing and retrying", self.name);
                match refresh().await {
                    Ok(()) => {
                        // Retry once with the new token. If this STILL returns
                        // 401, do NOT invalidate — could be transient server
                        // propagation delay. Let the outer rotation/fallback
                        // handle it; the next request cycle will try again.
                        return retry_with_backoff(
                            || async {
                                let body = self.encode_body(&openai_request)?;
                                let response = self
                                    .client
                                    .post(self.send_url())
                                    .headers(self.headers()?)
                                    .json(&body)
                                    .send()
                                    .await?;
                                if !response.status().is_success() {
                                    return Err(self.handle_error(response).await);
                                }
                                let openai_response: OpenAIResponse = response.json().await?;
                                Ok(self.from_openai_response(openai_response))
                            },
                            &retry_config,
                        )
                        .await;
                    }
                    Err(msg) => {
                        tracing::error!("{} auth refresh failed: {}", self.name, msg);
                        // Only invalidate when the refresh_token itself is dead
                        // (HTTP 400 from the token endpoint). Other errors
                        // (network, WAF, transient 5xx) must NOT invalidate.
                        if msg.contains("HTTP 400")
                            && let Some(ref invalidate) = self.auth_invalidate_fn
                        {
                            tracing::warn!(
                                "{} refresh_token permanently dead — invalidating account",
                                self.name
                            );
                            invalidate();
                        }
                    }
                }
            }

            tracing::error!("OpenAI API request failed: {}", e);
        }

        result
    }

    async fn stream(&self, request: LLMRequest) -> Result<ProviderStream> {
        use super::retry::retry_with_backoff;

        let model = request.model.clone();
        let message_count = request.messages.len();

        // Proactive pacing: stay under provider rate limits
        if let Some(ref limiter) = self.rate_limiter {
            let waited = limiter.wait().await;
            if !waited.is_zero() {
                tracing::debug!(
                    "Rate limiter: waited {:?} before streaming request to {}",
                    waited,
                    self.base_url
                );
            }
        }

        tracing::info!(
            "{} streaming request: model={}, messages={}, base_url={}",
            self.name(),
            model,
            message_count,
            self.base_url
        );

        let mut openai_request = self.to_openai_request(request);
        openai_request.stream = Some(true);
        openai_request.stream_options = Some(StreamOptions {
            include_usage: true,
        });

        let tools_count = openai_request.tools.as_ref().map(|t| t.len()).unwrap_or(0);

        // Count input tokens via tiktoken (cl100k_base) to monitor context window usage.
        // Each message: content tokens + serialized tool_calls tokens + 4 overhead per message.
        let message_tokens: usize = openai_request
            .messages
            .iter()
            .map(|m| {
                let content = m
                    .content
                    .as_ref()
                    .map(|v| {
                        let s = v.as_str().unwrap_or("");
                        count_message_tokens(s)
                    })
                    .unwrap_or(4);
                let tool_calls = m
                    .tool_calls
                    .as_ref()
                    .map(|tc| count_tokens(&serde_json::to_string(tc).unwrap_or_default()))
                    .unwrap_or(0);
                content + tool_calls
            })
            .sum();
        let tool_schema_tokens = openai_request
            .tools
            .as_ref()
            .map(|tools| count_tokens(&serde_json::to_string(tools).unwrap_or_default()))
            .unwrap_or(0);
        let total_input_tokens = message_tokens + tool_schema_tokens;
        let context_pct = (total_input_tokens as f32 / 200_000.0 * 100.0).round() as u32;
        tracing::debug!(
            "OpenAI stream request: ~{} input tokens ({}% of 200k window) — {} messages, {} tool schemas",
            total_input_tokens,
            context_pct,
            openai_request.messages.len(),
            tools_count
        );

        let retry_config = self.retry_config(&model);

        // Retry the stream connection establishment
        let mut response = retry_with_backoff(
            || async {
                let body = self.encode_body(&openai_request)?;
                let response = self
                    .client
                    .post(self.send_url())
                    .headers(self.headers()?)
                    .json(&body)
                    .send()
                    .await?;

                tracing::debug!("OpenAI response status: {}", response.status());

                if !response.status().is_success() {
                    return Err(self.handle_error(response).await);
                }

                Ok(response)
            },
            &retry_config,
        )
        .await;

        // Resilient fallback: if the API rejected max_tokens / max_completion_tokens,
        // swap the fields and retry once.
        if let Err(ref e) = response
            && is_token_field_mismatch(&e.to_string())
        {
            tracing::warn!(
                "Token field mismatch for model {} (stream), retrying with swapped fields",
                model
            );
            openai_request.swap_token_fields();
            response = retry_with_backoff(
                || async {
                    let body = self.encode_body(&openai_request)?;
                    let r = self
                        .client
                        .post(self.send_url())
                        .headers(self.headers()?)
                        .json(&body)
                        .send()
                        .await?;
                    if !r.status().is_success() {
                        return Err(self.handle_error(r).await);
                    }
                    Ok(r)
                },
                &retry_config,
            )
            .await;
        }

        // Auth-refresh fallback: on 401/403, call the refresh hook once and
        // retry a single time. Mirrors the non-streaming path.
        if let Err(ref e) = response
            && Self::is_auth_error(e)
            && let Some(ref refresh) = self.auth_refresh_fn
        {
            tracing::warn!("{} stream auth error — refreshing and retrying", self.name);
            match refresh().await {
                Ok(()) => {
                    // Retry once with the new token. If it STILL returns 401,
                    // do NOT invalidate — same rationale as non-streaming path.
                    response = retry_with_backoff(
                        || async {
                            let body = self.encode_body(&openai_request)?;
                            let r = self
                                .client
                                .post(self.send_url())
                                .headers(self.headers()?)
                                .json(&body)
                                .send()
                                .await?;
                            if !r.status().is_success() {
                                return Err(self.handle_error(r).await);
                            }
                            Ok(r)
                        },
                        &retry_config,
                    )
                    .await;
                }
                Err(msg) => {
                    tracing::error!("{} stream auth refresh failed: {}", self.name, msg);
                    // Only invalidate when the refresh_token itself is dead
                    // (HTTP 400 from the token endpoint).
                    if msg.contains("HTTP 400")
                        && let Some(ref invalidate) = self.auth_invalidate_fn
                    {
                        tracing::warn!(
                            "{} refresh_token permanently dead — invalidating account",
                            self.name
                        );
                        invalidate();
                    }
                }
            }
        }
        let response = response?;

        // Parse Server-Sent Events stream - return Vec to emit multiple events like Anthropic
        let byte_stream = response.bytes_stream();
        let buffer = std::sync::Arc::new(std::sync::Mutex::new(String::new()));

        // Accumulated state for a single streamed tool call
        #[derive(Debug, Clone, Default)]
        struct ToolCallAccum {
            id: String,
            name: String,
            arguments: String,
        }

        /// State persisted across SSE chunks via Arc<Mutex<_>>
        struct StreamState {
            emitted_message_start: bool,
            emitted_content_start: bool,
            /// Matching ContentBlockStop for the text block at index 0 has been emitted
            emitted_content_stop: bool,
            /// True once we've received real content via `delta` field
            seen_delta_content: bool,
            /// Index -> accumulated tool call
            tool_calls: std::collections::HashMap<usize, ToolCallAccum>,
            /// True while inside a stripped block (think/reasoning/tools-v2)
            inside_think: bool,
            /// Index into STRIP_CLOSE_TAGS for the currently active block
            active_close_tag: usize,
            /// Bytes consumed while inside_think is true (no close tag found).
            /// If this exceeds the threshold, we abandon filtering and pass
            /// content through — the model likely hallucinated an open tag
            /// without a matching close (e.g. `<!-- tools-v2: ...` with no `-->`).
            think_bytes_consumed: usize,
            /// Bytes withheld from the previous chunk because they could be
            /// the start of a split open tag (e.g. chunk ended with `<!`,
            /// which could become `<!--`). Prepended to the next chunk so
            /// the open-tag finder can match across chunk boundaries.
            think_carry: String,
            /// Stashed stop_reason from finish_reason chunk, emitted with
            /// the final usage-only chunk (MiniMax/OpenAI include_usage flow).
            pending_stop_reason: Option<crate::brain::provider::types::StopReason>,
            /// Rolling buffer of the leading visible content. Used to catch
            /// a hallucinated `{"tool_calls":...}` JSON envelope that
            /// dialagram-style providers stream 1-3 chars at a time through
            /// `delta.content`. While `leak_probe` is a strict prefix of the
            /// known leak markers, content is buffered rather than emitted.
            /// Once the accumulator diverges from every marker, the buffer
            /// is flushed through the normal think-tag filter. If the full
            /// marker is matched, `leak_active` is set and ALL subsequent
            /// content deltas are dropped for the remainder of the turn.
            leak_probe: String,
            leak_active: bool,
            /// Rolling accumulator of every emitted TextDelta across the turn.
            /// Scanned at `finish_reason` for `<tool_call>` / `<function=`
            /// blocks that local GGUF/MLX backends sometimes drop into
            /// content instead of the structured `tool_calls` field. Kept
            /// separate from `leak_probe` because the leak filter rejects
            /// text before emission, whereas this tracks what the consumer
            /// actually received.
            response_text_accum: String,
            /// Content suppressed by the tool-call marker filter (probe
            /// + subsequent suppressed deltas). Never reaches the consumer
            /// as text — scanned at `finish_reason` by `extract_text_tool_calls`
            /// and promoted to real `ContentBlock::ToolUse` events. This is
            /// how local-Qwen `tool_call:{...}` / `<tool_call>…` leaks become
            /// indistinguishable from cloud providers' structured tool calls
            /// in the TUI: the raw JSON never flows to the display layer.
            tool_capture_buffer: String,
            /// Once a tool-call marker has been seen in the stream we flip
            /// this to `true` and silently route all subsequent content
            /// deltas into `tool_capture_buffer` until the turn ends.
            /// Without this the model's `<tool_call>...</tool_call>` or
            /// `tool_call:{...}` text flashes in the TUI exactly the way
            /// cloud providers' structured `tool_calls` never would.
            tool_block_active: bool,
            /// Trailing bytes carried across chunks so the marker detector
            /// can match tokens that span chunk boundaries. Local models
            /// stream 1-3 bytes per SSE event, so "tool_call:" arrives as
            /// "too" + "l_" + "call:" — no single chunk contains the full
            /// marker. Holding back a short suffix bridges the split,
            /// mirroring `think_carry`'s role for `<think>` tag detection.
            marker_carry: String,
        }

        // Detect whether this stream is talking to a local inference server
        // (llama.cpp / MLX / LM Studio / Ollama). Local Qwen / Kimi / DeepSeek
        // models leak tool calls as content text — we want to suppress those
        // markers from the display stream so the TUI renders only structured
        // `ContentBlock::ToolUse` events, matching the cloud-provider UX.
        let is_local_stream = crate::brain::provider::factory::is_local_base_url(&self.base_url);

        let state = std::sync::Arc::new(std::sync::Mutex::new(StreamState {
            emitted_message_start: false,
            emitted_content_start: false,
            emitted_content_stop: false,
            seen_delta_content: false,
            tool_calls: std::collections::HashMap::new(),
            inside_think: false,
            active_close_tag: 0,
            think_bytes_consumed: 0,
            think_carry: String::new(),
            pending_stop_reason: None,
            leak_probe: String::new(),
            leak_active: false,
            response_text_accum: String::new(),
            tool_capture_buffer: String::new(),
            tool_block_active: false,
            marker_carry: String::new(),
        }));

        let event_stream = byte_stream
            .map(move |chunk_result| -> Vec<std::result::Result<StreamEvent, ProviderError>> {
                match chunk_result {
                    Err(e) => vec![Err(ProviderError::StreamError(e.to_string()))],
                    Ok(chunk) => {
                        // GRANULAR LOG: Raw SSE chunk
                        let raw_text = String::from_utf8_lossy(&chunk);
                        tracing::debug!("[STREAM_RAW] SSE chunk: {}", raw_text.chars().take(500).collect::<String>());
                        if raw_text.contains("tool_calls") {
                            tracing::debug!("[STREAM_RAW] SSE chunk with tool_calls: {}", raw_text.chars().take(500).collect::<String>());
                        }

                        let mut buf = buffer.lock().expect("SSE buffer lock poisoned");
                        buf.push_str(&raw_text);

                        let mut events = Vec::new();
                        let mut st = state.lock().expect("SSE state lock");

                        // Process complete lines (terminated by \n)
                        while let Some(newline_pos) = buf.find('\n') {
                            let line = buf[..newline_pos].trim().to_string();
                            buf.drain(..=newline_pos);

                            if let Some(json_str) = line.strip_prefix("data: ") {
                                if json_str == "[DONE]" {
                                    // Close the text block first, if one is still open,
                                    // so helpers.rs can finalize it before tool events.
                                    if st.emitted_content_start && !st.emitted_content_stop {
                                        events.push(Ok(StreamEvent::ContentBlockStop { index: 0 }));
                                        st.emitted_content_stop = true;
                                    }
                                    // Flush any accumulated tool calls before DONE.
                                    // Emit a matching ContentBlockStop after every Start so
                                    // helpers.rs fires ToolStarted/ToolCompleted progress
                                    // events to the TUI (otherwise the tool cards stay
                                    // visually "stuck" forever).
                                    for (_idx, accum) in st.tool_calls.drain() {
                                        let input = serde_json::from_str(&accum.arguments)
                                            .unwrap_or_else(|_| serde_json::json!({}));
                                        tracing::info!(
                                            "[TOOL_EMIT] Flushing tool on DONE: id={}, name={}, args={}",
                                            accum.id, accum.name, &accum.arguments.chars().take(200).collect::<String>()
                                        );
                                        let tool_index = _idx + 1; // Offset to avoid collision with text block at index 0
                                        events.push(Ok(StreamEvent::ContentBlockStart {
                                            index: tool_index,
                                            content_block: ContentBlock::ToolUse {
                                                id: accum.id,
                                                name: accum.name,
                                                input,
                                            },
                                        }));
                                        events.push(Ok(StreamEvent::ContentBlockStop { index: tool_index }));
                                    }
                                    // If we still have a pending stop_reason (no usage-only chunk
                                    // arrived), emit MessageDelta with fallback usage now.
                                    if let Some(stop_reason) = st.pending_stop_reason.take() {
                                        tracing::info!("[STREAM_USAGE] Final usage (fallback on DONE): input={}, output=0", total_input_tokens);
                                        events.push(Ok(StreamEvent::MessageDelta {
                                            delta: crate::brain::provider::types::MessageDelta {
                                                stop_reason: Some(stop_reason),
                                                stop_sequence: None,
                                            },
                                            usage: crate::brain::provider::types::TokenUsage {
                                                input_tokens: total_input_tokens as u32,
                                                output_tokens: 0, ..Default::default() },
                                        }));
                                    }
                                    events.push(Ok(StreamEvent::MessageStop));
                                    continue;
                                }

                                // Check for z.ai/provider-specific inline errors (HTTP 200 with error in body)
                                if let Ok(raw) = serde_json::from_str::<serde_json::Value>(json_str)
                                    && let Some(status_msg) = raw.pointer("/base_resp/status_msg").and_then(|v| v.as_str())
                                {
                                    let status_code = raw.pointer("/base_resp/status_code").and_then(|v| v.as_u64()).unwrap_or(0);
                                    if status_code != 0 {
                                        tracing::error!("[STREAM_ERROR] Provider returned inline error: code={}, msg={}", status_code, status_msg);
                                        events.push(Err(ProviderError::ApiError {
                                            status: status_code as u16,
                                            message: status_msg.to_string(),
                                            error_type: Some("provider_error".to_string()),
                                        }));
                                        continue;
                                    }
                                }

                                // Check for generic `{error: {message, type, code}}` mid-stream.
                                // DashScope/qwen emits these when quota is hit mid-flight, and
                                // OpenAI uses the same shape for policy/tos violations.
                                if let Ok(raw) = serde_json::from_str::<serde_json::Value>(json_str)
                                    && let Some(err_obj) = raw.get("error").and_then(|v| v.as_object())
                                {
                                    let message = err_obj.get("message").and_then(|v| v.as_str()).unwrap_or("stream error").to_string();
                                    let err_type = err_obj.get("type").and_then(|v| v.as_str()).map(|s| s.to_string());
                                    // Status can arrive as `code` (OpenAI) or `http_code`
                                    // (qwen portal), and qwen serializes it as a string.
                                    // Parse both, preferring whichever is non-zero.
                                    let read_status = |key: &str| -> u16 {
                                        err_obj
                                            .get(key)
                                            .and_then(|v| {
                                                v.as_u64().map(|n| n as u16).or_else(|| {
                                                    v.as_str().and_then(|s| s.parse::<u16>().ok())
                                                })
                                            })
                                            .unwrap_or(0)
                                    };
                                    let code = {
                                        let c = read_status("code");
                                        if c != 0 { c } else { read_status("http_code") }
                                    };
                                    tracing::error!("[STREAM_ERROR] Inline SSE error: type={:?}, code={}, msg={}", err_type, code, message);
                                    // Map 429/quota-style messages to RateLimitExceeded so the
                                    // fallback chain kicks in immediately instead of retrying.
                                    let msg_lc = message.to_lowercase();
                                    let is_rate_limit = code == 429
                                        || err_type.as_deref() == Some("rate_limit_exceeded")
                                        || msg_lc.contains("rate limit")
                                        || msg_lc.contains("quota");
                                    // Qwen portal (and Cloudflare-fronted upstreams in general)
                                    // emits 529 / `overloaded_error` / 503 when the shared
                                    // cluster is temporarily saturated. It's the exact case
                                    // tool_loop's StreamError retry-then-fallback path exists
                                    // for: retry 3x with backoff, then swap to a healthy
                                    // provider. Mapping to StreamError routes it there —
                                    // mapping to ApiError{500} sent it to the catch-all that
                                    // bubbled straight to the TUI.
                                    let is_overloaded = code == 529
                                        || code == 503
                                        || err_type.as_deref() == Some("overloaded_error")
                                        || msg_lc.contains("overloaded")
                                        || msg_lc.contains("server cluster")
                                        || msg_lc.contains("high load");
                                    let pe = if is_rate_limit {
                                        ProviderError::RateLimitExceeded(message)
                                    } else if is_overloaded {
                                        ProviderError::StreamError(format!(
                                            "upstream overloaded ({}): {}",
                                            code, message
                                        ))
                                    } else {
                                        ProviderError::ApiError {
                                            status: if code == 0 { 500 } else { code },
                                            message,
                                            error_type: err_type,
                                        }
                                    };
                                    events.push(Err(pe));
                                    continue;
                                }

                                match serde_json::from_str::<OpenAIStreamChunk>(json_str) {
                                    Ok(chunk) => {
                                        // Emit MessageStart on first chunk with id
                                        if !st.emitted_message_start && !chunk.id.is_empty() {
                                            st.emitted_message_start = true;
                                            let model = chunk.model.clone().unwrap_or_default();
                                            events.push(Ok(StreamEvent::MessageStart {
                                                message: crate::brain::provider::types::StreamMessage {
                                                    id: chunk.id,
                                                    model,
                                                    role: Role::Assistant,
                                                    usage: crate::brain::provider::types::TokenUsage {
                                                        input_tokens: 0,
                                                        output_tokens: 0, ..Default::default() },
                                                },
                                            }));
                                        }

                                        // Get content from delta or message (MiniMax uses message).
                                        // IMPORTANT: Some providers (LM Studio, etc.) send the FULL
                                        // response in the final chunk's `message` field while `delta`
                                        // is absent. If we already received content via delta, we must
                                        // NOT fall back to `message` or we'll duplicate the entire text.
                                        let delta_content = chunk.choices.first()
                                            .and_then(|c| c.delta.as_ref())
                                            .and_then(|d| d.content.as_ref())
                                            .cloned();
                                        let content = if delta_content.is_some() {
                                            if delta_content.as_ref().is_some_and(|s| !s.is_empty()) {
                                                st.seen_delta_content = true;
                                            }
                                            delta_content
                                        } else if !st.seen_delta_content {
                                            // Only use message field if we've never seen delta content
                                            // (MiniMax always uses message, standard providers don't)
                                            chunk.choices.first()
                                                .and_then(|c| c.message.as_ref())
                                                .and_then(|d| d.content.as_ref())
                                                .cloned()
                                        } else {
                                            None
                                        };

                                        // Get streaming tool_calls from delta or message
                                        let tool_calls = chunk.choices.first()
                                            .and_then(|c| c.delta.as_ref().or(c.message.as_ref()))
                                            .and_then(|d| d.tool_calls.as_ref());

                                        // Accumulate tool calls across chunks
                                        // OpenAI streaming sends: chunk1={index,id,type,name,args:""}, chunk2..N={index,args:"<fragment>"}
                                        if let Some(tc_list) = tool_calls {
                                            for tc_item in tc_list {
                                                let idx = tc_item.index;
                                                let accum = st.tool_calls.entry(idx).or_default();

                                                // First chunk for this index carries id + name
                                                if let Some(ref id) = tc_item.id
                                                    && !id.is_empty() {
                                                        accum.id = id.clone();
                                                    }
                                                if let Some(ref func) = tc_item.function {
                                                    if let Some(ref name) = func.name
                                                        && !name.is_empty() {
                                                            accum.name = name.clone();
                                                        }
                                                    // Append argument fragment
                                                    if let Some(ref args) = func.arguments {
                                                        accum.arguments.push_str(args);
                                                    }
                                                }

                                                tracing::debug!(
                                                    "[TOOL_ACCUM] idx={}, id={}, name={}, args_len={}, args_tail={}",
                                                    idx, accum.id, accum.name, accum.arguments.len(),
                                                    accum.arguments.chars().rev().take(60).collect::<String>().chars().rev().collect::<String>()
                                                );
                                            }
                                        }

                                        // Check finish_reason — emit accumulated tool calls when done
                                        let finish_reason_str = chunk.choices.first()
                                            .and_then(|c| c.finish_reason.as_ref());

                                        // Flush accumulated tool calls on any terminal finish_reason.
                                        // Some providers (MiniMax) send "stop" even with tool_calls.
                                        if finish_reason_str.is_some() && !st.tool_calls.is_empty() {
                                                // Close the text block first if one is still open so
                                                // helpers.rs finalizes it before the tool blocks.
                                                if st.emitted_content_start && !st.emitted_content_stop {
                                                    events.push(Ok(StreamEvent::ContentBlockStop { index: 0 }));
                                                    st.emitted_content_stop = true;
                                                }
                                                // Emit all accumulated tool calls. Each Start MUST be
                                                // followed by a matching Stop so helpers.rs can fire
                                                // ToolStarted/ToolCompleted progress events — otherwise
                                                // the TUI tool cards stay visually stuck forever.
                                                for (idx, accum) in st.tool_calls.drain() {
                                                    let input = serde_json::from_str(&accum.arguments)
                                                        .unwrap_or_else(|e| {
                                                            tracing::warn!(
                                                                "[TOOL_EMIT] Failed to parse accumulated args for '{}': {} | args: {}",
                                                                accum.name, e, &accum.arguments.chars().take(300).collect::<String>()
                                                            );
                                                            serde_json::json!({})
                                                        });
                                                    tracing::info!(
                                                        "[TOOL_EMIT] Emitting tool call: idx={}, id={}, name={}, args_len={}",
                                                        idx, accum.id, accum.name, accum.arguments.len()
                                                    );
                                                    let tool_index = idx + 1; // Offset by 1 to avoid collision with text block at index 0
                                                    events.push(Ok(StreamEvent::ContentBlockStart {
                                                        index: tool_index,
                                                        content_block: ContentBlock::ToolUse {
                                                            id: accum.id,
                                                            name: accum.name,
                                                            input,
                                                        },
                                                    }));
                                                    events.push(Ok(StreamEvent::ContentBlockStop { index: tool_index }));
                                                }
                                            }

                                        // Emit text content, filtering <think>...</think> reasoning blocks
                                        if let Some(ref c) = content {
                                            // Defensive: qwen-3.6-plus-thinking (and other thinking
                                            // models) occasionally hallucinate OpenAI tool_call
                                            // envelopes as plain text content (e.g.
                                            // `{"tool_calls":[{"id"...`). The real tool calls come
                                            // through `delta.tool_calls` — any such text in
                                            // `delta.content` is pure noise. Drop it outright.
                                            //
                                            // Detection runs over the ROLLING accumulator because
                                            // dialagram-style providers stream 1-3 chars per delta
                                            // and the single-chunk prefix check would never fire.
                                            // Strategy: while the trimmed accumulator is still a
                                            // strict prefix of a known leak marker, buffer the
                                            // content. Once the full marker matches → drop the
                                            // buffer and flip leak_active for the rest of the turn.
                                            // If the accumulator diverges → flush the buffer
                                            // through the normal think-tag filter.
                                            const LEAK_MARKERS: &[&str] = &[
                                                "{\"tool_calls\"",
                                                "{ \"tool_calls\"",
                                            ];

                                            // Once we know the turn is leaking, drop everything.
                                            let drop_all = st.leak_active;
                                            let mut to_emit: Option<String> = None;
                                            if drop_all {
                                                if !c.is_empty() {
                                                    tracing::debug!(
                                                        "[STREAM_FILTER] Suppressing {} chars of content during active tool_calls leak",
                                                        c.len()
                                                    );
                                                }
                                            } else {
                                                st.leak_probe.push_str(c);
                                                let probe_trimmed = st.leak_probe.trim_start();
                                                let full_match = LEAK_MARKERS
                                                    .iter()
                                                    .any(|m| probe_trimmed.starts_with(m));
                                                if full_match {
                                                    tracing::warn!(
                                                        "[STREAM_FILTER] Dropping hallucinated tool_calls JSON across accumulated content ({} chars buffered)",
                                                        st.leak_probe.len()
                                                    );
                                                    st.leak_active = true;
                                                    st.leak_probe.clear();
                                                } else {
                                                    // Is the trimmed accum still a viable prefix
                                                    // of some marker? If so, keep buffering.
                                                    // Also allow buffering when the accum is empty
                                                    // after trim (pure leading whitespace).
                                                    let still_prefix = probe_trimmed.is_empty()
                                                        || LEAK_MARKERS.iter().any(|m| {
                                                            m.starts_with(probe_trimmed)
                                                        });
                                                    if still_prefix {
                                                        // Keep withholding — bounded by marker len.
                                                        // Safety cap so a legitimate response that
                                                        // happens to start with "{" doesn't get
                                                        // buffered forever.
                                                        if st.leak_probe.len() > 64 {
                                                            to_emit = Some(std::mem::take(&mut st.leak_probe));
                                                        }
                                                    } else {
                                                        // Diverged — flush the buffer as normal content.
                                                        to_emit = Some(std::mem::take(&mut st.leak_probe));
                                                    }
                                                }
                                            }

                                            if let Some(ref flushed) = to_emit {
                                                let (mut inside, mut close_idx, mut consumed) =
                                                    (st.inside_think, st.active_close_tag, st.think_bytes_consumed);
                                                let mut carry = std::mem::take(&mut st.think_carry);
                                                let (filtered, reasoning_from_think) = filter_think_tags(
                                                    flushed,
                                                    &mut inside,
                                                    &mut close_idx,
                                                    &mut consumed,
                                                    &mut carry,
                                                );
                                                st.inside_think = inside;
                                                st.active_close_tag = close_idx;
                                                st.think_bytes_consumed = consumed;
                                                st.think_carry = carry;

                                                // Content inside `<think>…</think>` is promoted to a
                                                // `ReasoningDelta` event so downstream treats it like
                                                // the OpenAI `reasoning_content` field — helpers.rs
                                                // accumulates it into `reasoning_buf`, emits a
                                                // `ReasoningChunk` progress event the TUI renders as
                                                // live "Thinking…" text, and lands it as `details` on
                                                // the final `DisplayMessage`. Without this, models
                                                // like Qwen that emit `<think>` inline would have
                                                // their thinking silently discarded.
                                                if !reasoning_from_think.is_empty() {
                                                    if !st.emitted_content_start {
                                                        st.emitted_content_start = true;
                                                        events.push(Ok(StreamEvent::ContentBlockStart {
                                                            index: 0,
                                                            content_block: ContentBlock::Text { text: String::new() },
                                                        }));
                                                    }
                                                    events.push(Ok(StreamEvent::ContentBlockDelta {
                                                        index: 0,
                                                        delta: ContentDelta::ReasoningDelta {
                                                            text: reasoning_from_think,
                                                        },
                                                    }));
                                                }

                                                // For local providers (llama.cpp/MLX/etc.) the model often
                                                // leaks tool calls as content text. Partition the filtered
                                                // text at the first tool-call marker: everything before
                                                // stays visible (legitimate intermediate text like
                                                // "I'll check git status. "), everything from the marker
                                                // onwards is silently captured and promoted to a
                                                // structured `ContentBlock::ToolUse` at `finish_reason`.
                                                // Once the capture flag flips, subsequent deltas never
                                                // reach the consumer as text — matching the UX of every
                                                // other provider which emits structured tool_calls.
                                                //
                                                // Cross-chunk handling: local models stream 1-3 bytes per
                                                // SSE event, so "tool_call:" arrives as "too" + "l_" +
                                                // "call:" and no individual chunk contains the full
                                                // marker. Before scanning, prepend `marker_carry` (bytes
                                                // held back from the previous chunk's trailing prefix).
                                                // After scanning, hold back any suffix that still looks
                                                // like the start of a marker so the next chunk can close
                                                // the match.
                                                const TOOL_MARKERS: &[&str] = &[
                                                    "<tool_call>",
                                                    "<function=",
                                                    "tool_call:",
                                                    "\"tool_calls\"",
                                                    "\"tool_call\"",
                                                ];
                                                let mut display_text: String = String::new();
                                                if is_local_stream {
                                                    if st.tool_block_active {
                                                        // Already capturing — all further content is tool body.
                                                        st.tool_capture_buffer.push_str(&filtered);
                                                    } else {
                                                        // Prepend carry so tokens split across chunk
                                                        // boundaries still match as one unit.
                                                        let mut working = std::mem::take(&mut st.marker_carry);
                                                        working.push_str(&filtered);

                                                        let first = TOOL_MARKERS
                                                            .iter()
                                                            .filter_map(|m| working.find(m).map(|p| (p, *m)))
                                                            .min_by_key(|(p, _)| *p);

                                                        if let Some((pos, marker)) = first {
                                                            let before: String = working[..pos].to_string();
                                                            let after = &working[pos..];
                                                            st.tool_capture_buffer.push_str(after);
                                                            st.tool_block_active = true;
                                                            display_text = before;
                                                            tracing::info!(
                                                                "[STREAM_FILTER] Tool-call marker {:?} detected — routing {} bytes to capture buffer",
                                                                marker, after.len()
                                                            );
                                                        } else {
                                                            // No full marker match. Hold back any trailing
                                                            // suffix that's a viable prefix of some marker
                                                            // so the next chunk can finish the match.
                                                            let tail_keep = tool_marker_prefix_len(
                                                                &working,
                                                                TOOL_MARKERS,
                                                            );
                                                            if tail_keep >= working.len() {
                                                                // Entire working string is a viable prefix
                                                                // — hold it all, emit nothing this tick.
                                                                st.marker_carry = working;
                                                            } else if tail_keep > 0 {
                                                                let split = working.len() - tail_keep;
                                                                display_text = working[..split].to_string();
                                                                st.marker_carry =
                                                                    working[split..].to_string();
                                                            } else {
                                                                display_text = working;
                                                            }
                                                        }
                                                    }
                                                } else {
                                                    display_text = filtered.clone();
                                                }

                                                if !display_text.is_empty() {
                                                    if !st.emitted_content_start {
                                                        st.emitted_content_start = true;
                                                        events.push(Ok(StreamEvent::ContentBlockStart {
                                                            index: 0,
                                                            content_block: ContentBlock::Text { text: String::new() },
                                                        }));
                                                    }

                                                    st.response_text_accum.push_str(&display_text);
                                                    events.push(Ok(StreamEvent::ContentBlockDelta {
                                                        index: 0,
                                                        delta: ContentDelta::TextDelta {
                                                            text: display_text,
                                                        },
                                                    }));
                                                } else if !st.emitted_content_start
                                                    && flushed.is_empty()
                                                    && !st.tool_block_active
                                                {
                                                    st.emitted_content_start = true;
                                                    events.push(Ok(StreamEvent::ContentBlockStart {
                                                        index: 0,
                                                        content_block: ContentBlock::Text { text: String::new() },
                                                    }));
                                                }
                                            }
                                        }

                                        // Extract reasoning_content (MiniMax/dialagram thinking process).
                                        // Providers like dialagram `qwen-3.6-plus-thinking` stream the entire
                                        // reasoning BEFORE any text content arrives. helpers.rs silently drops
                                        // ContentBlockDelta at an index with no matching ContentBlockStart, so
                                        // we must open the text block at index 0 first — even if nothing has
                                        // been written to it yet — so the ReasoningDelta is actually forwarded
                                        // to the TUI via the ReasoningChunk progress event.
                                        let reasoning = chunk.choices.first()
                                            .and_then(|c| c.delta.as_ref())
                                            .and_then(|d| d.reasoning_content.as_ref())
                                            .cloned();
                                        if let Some(rc) = reasoning && !rc.is_empty() {
                                            if !st.emitted_content_start {
                                                st.emitted_content_start = true;
                                                events.push(Ok(StreamEvent::ContentBlockStart {
                                                    index: 0,
                                                    content_block: ContentBlock::Text { text: String::new() },
                                                }));
                                            }
                                            events.push(Ok(StreamEvent::ContentBlockDelta {
                                                index: 0,
                                                delta: ContentDelta::ReasoningDelta {
                                                    text: rc,
                                                },
                                            }));
                                        }

                                        // Emit MessageDelta when finish_reason is present.
                                        // Do NOT emit MessageStop here — providers that support
                                        // stream_options.include_usage (MiniMax, OpenAI) send a
                                        // final usage-only chunk AFTER this one. We handle
                                        // MessageStop on [DONE] or the usage-only chunk below.
                                        if let Some(reason) = finish_reason_str {
                                            // If we were still withholding a leak-probe buffer
                                            // waiting to confirm/deny a tool_calls envelope and
                                            // the turn is ending without it ever matching, flush
                                            // the buffered content as legitimate text so short
                                            // responses that happen to begin with `{` aren't lost.
                                            if !st.leak_active && !st.leak_probe.is_empty() {
                                                let flushed = std::mem::take(&mut st.leak_probe);
                                                let (mut inside, mut close_idx, mut consumed) =
                                                    (st.inside_think, st.active_close_tag, st.think_bytes_consumed);
                                                let mut carry = std::mem::take(&mut st.think_carry);
                                                let (filtered, reasoning_from_think) = filter_think_tags(
                                                    &flushed,
                                                    &mut inside,
                                                    &mut close_idx,
                                                    &mut consumed,
                                                    &mut carry,
                                                );
                                                st.inside_think = inside;
                                                st.active_close_tag = close_idx;
                                                st.think_bytes_consumed = consumed;
                                                st.think_carry = carry;
                                                if !reasoning_from_think.is_empty() {
                                                    if !st.emitted_content_start {
                                                        st.emitted_content_start = true;
                                                        events.push(Ok(StreamEvent::ContentBlockStart {
                                                            index: 0,
                                                            content_block: ContentBlock::Text { text: String::new() },
                                                        }));
                                                    }
                                                    events.push(Ok(StreamEvent::ContentBlockDelta {
                                                        index: 0,
                                                        delta: ContentDelta::ReasoningDelta {
                                                            text: reasoning_from_think,
                                                        },
                                                    }));
                                                }
                                                if !filtered.is_empty() {
                                                    if !st.emitted_content_start {
                                                        st.emitted_content_start = true;
                                                        events.push(Ok(StreamEvent::ContentBlockStart {
                                                            index: 0,
                                                            content_block: ContentBlock::Text { text: String::new() },
                                                        }));
                                                    }
                                                    st.response_text_accum.push_str(&filtered);
                                                    events.push(Ok(StreamEvent::ContentBlockDelta {
                                                        index: 0,
                                                        delta: ContentDelta::TextDelta { text: filtered },
                                                    }));
                                                }
                                            }
                                            // Fallback: scan accumulated text for tool-call blocks
                                            // a local GGUF/MLX backend dropped into content. Two
                                            // sources to check:
                                            //   - `tool_capture_buffer` — marker-suppressed content
                                            //     (primary source for local providers; display never
                                            //     saw it)
                                            //   - `response_text_accum` — what did reach display,
                                            //     as a safety net in case the partitioning missed a
                                            //     marker (e.g. split across chunk boundaries before
                                            //     the first marker match).
                                            // Only fires when the structured `tool_calls` path
                                            // produced nothing, so providers that already emit
                                            // proper tool_calls pay no cost beyond the substring check.
                                            let has_markers_in_accum = st.response_text_accum.contains("<tool_call>")
                                                || st.response_text_accum.contains("<function=")
                                                || st.response_text_accum.contains("tool_call:")
                                                || st.response_text_accum.contains("\"tool_calls\"");
                                            let has_capture = !st.tool_capture_buffer.is_empty();
                                            if st.tool_calls.is_empty() && (has_markers_in_accum || has_capture)
                                            {
                                                let mut combined = st.tool_capture_buffer.clone();
                                                if has_markers_in_accum {
                                                    // Any markers still left in the display accum
                                                    // get scanned too — prepend so positional order
                                                    // roughly matches the stream order.
                                                    let mut prefix = st.response_text_accum.clone();
                                                    prefix.push('\n');
                                                    prefix.push_str(&combined);
                                                    combined = prefix;
                                                }
                                                let (recovered, _cleaned) =
                                                    extract_text_tool_calls(&combined);
                                                if !recovered.is_empty() {
                                                    tracing::info!(
                                                        "Recovered {} streaming tool call(s) from text content (local-model fallback; capture_bytes={}, display_markers={})",
                                                        recovered.len(),
                                                        st.tool_capture_buffer.len(),
                                                        has_markers_in_accum,
                                                    );
                                                    // Close the text block first so helpers.rs
                                                    // finalizes it before the tool blocks arrive.
                                                    if st.emitted_content_start && !st.emitted_content_stop {
                                                        events.push(Ok(StreamEvent::ContentBlockStop { index: 0 }));
                                                        st.emitted_content_stop = true;
                                                    }
                                                    for (tc_idx, (name, input)) in recovered.into_iter().enumerate() {
                                                        let tool_index = tc_idx + 1;
                                                        events.push(Ok(StreamEvent::ContentBlockStart {
                                                            index: tool_index,
                                                            content_block: ContentBlock::ToolUse {
                                                                id: format!("call_text_{}", tc_idx),
                                                                name,
                                                                input,
                                                            },
                                                        }));
                                                        events.push(Ok(StreamEvent::ContentBlockStop { index: tool_index }));
                                                    }
                                                }
                                            }
                                            // Close the text block (no tool path above will have
                                            // handled this if there were no tool calls to flush).
                                            if st.emitted_content_start && !st.emitted_content_stop {
                                                events.push(Ok(StreamEvent::ContentBlockStop { index: 0 }));
                                                st.emitted_content_stop = true;
                                            }
                                            let (raw_input, raw_output, raw_cache_read, raw_cache_create) = if let Some(ref usage) = chunk.usage {
                                                (
                                                    usage.prompt_tokens.unwrap_or(0),
                                                    usage.completion_tokens.unwrap_or(0),
                                                    usage.effective_cache_read(),
                                                    usage.cache_creation_input_tokens.unwrap_or(0),
                                                )
                                            } else {
                                                (0, 0, 0, 0)
                                            };

                                            let stop_reason = Some(match reason.as_str() {
                                                "stop" => crate::brain::provider::types::StopReason::EndTurn,
                                                "length" => crate::brain::provider::types::StopReason::MaxTokens,
                                                "tool_calls" | "function_call" => crate::brain::provider::types::StopReason::ToolUse,
                                                _ => crate::brain::provider::types::StopReason::EndTurn,
                                            });

                                            // If this chunk already carries real usage (some
                                            // providers inline it), emit immediately + stop.
                                            if raw_input > 0 || raw_output > 0 {
                                                tracing::info!(
                                                    "[STREAM_USAGE] Final usage (inline): input={}, output={}, cache_read={}, cache_create={}",
                                                    raw_input, raw_output, raw_cache_read, raw_cache_create
                                                );
                                                events.push(Ok(StreamEvent::MessageDelta {
                                                    delta: crate::brain::provider::types::MessageDelta {
                                                        stop_reason,
                                                        stop_sequence: None,
                                                    },
                                                    usage: crate::brain::provider::types::TokenUsage {
                                                        input_tokens: raw_input,
                                                        output_tokens: raw_output,
                                                        cache_creation_tokens: raw_cache_create,
                                                        cache_read_tokens: raw_cache_read,
                                                        ..Default::default()
                                                    },
                                                }));
                                                events.push(Ok(StreamEvent::MessageStop));
                                            } else {
                                                // Stash stop_reason — we'll emit the final MessageDelta
                                                // with real usage once the usage-only chunk arrives.
                                                st.pending_stop_reason = stop_reason;
                                            }
                                        }

                                        // Handle usage-only chunk: choices is empty, usage has
                                        // real token counts. MiniMax and OpenAI send this as
                                        // the final chunk when stream_options.include_usage=true.
                                        if chunk.choices.is_empty()
                                            && let Some(ref usage) = chunk.usage {
                                                let input = usage.prompt_tokens.unwrap_or(0);
                                                let output = usage.completion_tokens.unwrap_or(0);
                                                let cache_read = usage.effective_cache_read();
                                                let cache_create = usage.cache_creation_input_tokens.unwrap_or(0);
                                                let reasoning = usage.reasoning_tokens();
                                                if input > 0 || output > 0 {
                                                    tracing::info!(
                                                        "[STREAM_USAGE] Final usage: input={}, output={}, cache_read={}, cache_create={}, reasoning={}",
                                                        input, output, cache_read, cache_create, reasoning
                                                    );
                                                    events.push(Ok(StreamEvent::MessageDelta {
                                                        delta: crate::brain::provider::types::MessageDelta {
                                                            stop_reason: st.pending_stop_reason.take(),
                                                            stop_sequence: None,
                                                        },
                                                        usage: crate::brain::provider::types::TokenUsage {
                                                            input_tokens: input,
                                                            output_tokens: output,
                                                            cache_creation_tokens: cache_create,
                                                            cache_read_tokens: cache_read,
                                                            ..Default::default()
                                                        },
                                                    }));
                                                    events.push(Ok(StreamEvent::MessageStop));
                                                }
                                        }
                                    }
                                    Err(e) => {
                                        let json_preview = json_str.chars().take(300).collect::<String>();
                                        tracing::warn!(
                                            "[STREAM_PARSE] Failed to parse chunk: {} | Raw: {}",
                                            e, json_preview
                                        );
                                    }
                                }
                            }
                        }

                        // ── Non-streaming fallback ──────────────────
                        // Some upstreams (OpenRouter Trinity, Venice) return a
                        // plain JSON blob instead of SSE. Detect and synthesize
                        // the same stream events the SSE parser would produce.
                        if events.is_empty()
                            && !st.emitted_message_start
                            && super::nonstream_compat::is_nonstream_response(&buf)
                            && let Some(synth) = super::nonstream_compat::synthesize_stream_events(&buf)
                        {
                            st.emitted_message_start = true;
                            st.emitted_content_start = true;
                            st.emitted_content_stop = true;
                            buf.clear();
                            events.extend(synth);
                        }

                        if events.is_empty() {
                            vec![Ok(StreamEvent::Ping)]
                        } else {
                            events
                        }
                    }
                }
            })
            .flat_map(futures::stream::iter);

        Ok(Box::pin(event_stream))
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn supports_vision(&self) -> bool {
        self.vision_model.is_some()
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn base_url(&self) -> Option<&str> {
        Some(&self.base_url)
    }

    fn default_model(&self) -> &str {
        self.custom_default_model.as_deref().unwrap_or_else(|| {
            tracing::error!(
                "No default_model configured for provider '{}' — check config.toml",
                self.name
            );
            "MISSING_MODEL"
        })
    }

    fn supported_models(&self) -> Vec<String> {
        vec![
            "gpt-4-turbo-preview".to_string(),
            "gpt-4".to_string(),
            "gpt-4-32k".to_string(),
            "gpt-3.5-turbo".to_string(),
            "gpt-3.5-turbo-16k".to_string(),
        ]
    }

    async fn fetch_models(&self) -> Vec<String> {
        // Derive models URL from base_url (replace /chat/completions with /models)
        let models_url = self.base_url.replace("/chat/completions", "/models");

        #[derive(Deserialize)]
        struct ModelEntry {
            id: String,
        }
        #[derive(Deserialize)]
        struct ModelsResponse {
            data: Vec<ModelEntry>,
        }

        let headers = match self.headers() {
            Ok(h) => h,
            Err(_) => return self.supported_models(),
        };
        match self.client.get(&models_url).headers(headers).send().await {
            Ok(resp) if resp.status().is_success() => match resp.json::<ModelsResponse>().await {
                Ok(body) => {
                    let mut models: Vec<String> = body.data.into_iter().map(|m| m.id).collect();
                    models.sort();
                    if models.is_empty() {
                        return self.supported_models();
                    }
                    models
                }
                Err(_) => self.supported_models(),
            },
            _ => self.supported_models(),
        }
    }

    fn configured_context_window(&self) -> Option<u32> {
        self.configured_context_window
    }

    fn context_window(&self, model: &str) -> Option<u32> {
        // User-configured value takes priority over model-name heuristics
        if let Some(cw) = self.configured_context_window {
            return Some(cw);
        }
        let m = model.to_lowercase();
        // gpt-5 family
        if m.starts_with("gpt-5") {
            return Some(1_047_576); // 1M tokens
        }
        // gpt-4.1 family
        if m.starts_with("gpt-4.1") {
            return Some(1_047_576); // 1M tokens
        }
        // o-series reasoning models
        if m.starts_with("o4") || m.starts_with("o3") {
            return Some(200_000);
        }
        if m.starts_with("o1") {
            return Some(200_000);
        }
        // gpt-4o family
        if m.starts_with("gpt-4o") {
            return Some(128_000);
        }
        match model {
            "gpt-4-turbo" | "gpt-4-turbo-preview" => Some(128_000),
            "gpt-4" => Some(8_192),
            "gpt-4-32k" => Some(32_768),
            "gpt-3.5-turbo" => Some(16_384),
            "gpt-3.5-turbo-16k" => Some(16_384),
            _ => None,
        }
    }

    fn calculate_cost(&self, model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
        // Always load fresh from disk — avoids stale OnceLock cache
        // that may have been initialized before usage_pricing.toml existed
        crate::pricing::PricingConfig::load().calculate_cost(model, input_tokens, output_tokens)
    }
}

/// Returns true if this model requires `max_completion_tokens` instead of `max_tokens`.
/// Newer OpenAI models (gpt-4.1-*, gpt-5-*, o1-*, o3-*) reject `max_tokens`.
/// Qwen thinking models also need this — when `max_tokens` is sent, DashScope
/// treats it as a text-only cap and reasoning tokens eat from a separate (tiny)
/// default budget, causing the model to stop after a handful of output tokens.
pub(crate) fn uses_max_completion_tokens(model: &str) -> bool {
    let m = model.to_lowercase();
    m.starts_with("gpt-4.1")
        || m.starts_with("gpt-5")
        || m.starts_with("o1")
        || m.starts_with("o3")
        || m.starts_with("o4")
        || m.contains("thinking")
}

// ============================================================================
// OpenAI API Types
// ============================================================================
// Anthropic-format request for OpenRouter (enables prompt caching)
// ============================================================================

/// Anthropic-style request for OpenRouter when routing to Anthropic models.
/// OpenRouter accepts this format and passes cache_control through to Anthropic.
#[derive(Debug, Clone, Serialize)]
struct AnthropicORRequest {
    model: String,
    messages: Vec<AnthropicORMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<Vec<AnthropicORSystemBlock>>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicORTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

/// System content block with cache_control support.
#[derive(Debug, Clone, Serialize)]
struct AnthropicORSystemBlock {
    r#type: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<AnthropicORCacheControl>,
}

/// Message in Anthropic format with content blocks.
#[derive(Debug, Clone, Serialize)]
struct AnthropicORMessage {
    role: String,
    content: Vec<AnthropicORContentBlock>,
}

/// Content block for messages (text, tool_use, tool_result).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicORContentBlock {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<AnthropicORCacheControl>,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

/// Tool definition with cache_control support.
#[derive(Debug, Clone, Serialize)]
struct AnthropicORTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<AnthropicORCacheControl>,
}

/// Ephemeral cache control marker.
#[derive(Debug, Clone, Serialize)]
struct AnthropicORCacheControl {
    r#type: String,
}

// ============================================================================
// OpenAI-compatible request/response types
// ============================================================================

#[derive(Debug, Clone, Serialize)]
struct OpenAIRequest {
    model: String,
    messages: Vec<OpenAIMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    /// Legacy token limit field — used by older OpenAI models (gpt-4o, gpt-3.5, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    /// New token limit field — required by newer OpenAI models (gpt-4.1-*, gpt-5-*, o1-*, o3-*)
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAITool>>,
    /// Tells the model whether/how to call tools. "auto" = model decides.
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<serde_json::Value>,
    /// OpenRouter: request reasoning/thinking tokens in the response.
    #[serde(skip_serializing_if = "Option::is_none")]
    include_reasoning: Option<bool>,
}

impl OpenAIRequest {
    /// Swap max_tokens ↔ max_completion_tokens for retry after a 400 error.
    fn swap_token_fields(&mut self) {
        let old_max = self.max_tokens.take();
        let old_completion = self.max_completion_tokens.take();
        self.max_tokens = old_completion;
        self.max_completion_tokens = old_max;
    }
}

/// Returns true if the error message indicates a max_tokens / max_completion_tokens mismatch.
pub(crate) fn is_token_field_mismatch(msg: &str) -> bool {
    let m = msg.to_lowercase();
    (m.contains("max_tokens") || m.contains("max_completion_tokens")) && m.contains("unsupported")
}

#[derive(Debug, Clone, Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAIMessage {
    role: String,
    /// Either a plain string or an array of content parts (text + image_url).
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAIToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAIToolCall {
    id: String,
    r#type: String,
    function: OpenAIFunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAIFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Clone, Serialize)]
struct OpenAITool {
    r#type: String,
    function: OpenAIFunction,
}

#[derive(Debug, Clone, Serialize)]
struct OpenAIFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAIResponse {
    id: String,
    model: String,
    choices: Vec<OpenAIChoice>,
    usage: OpenAIUsage,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct OpenAIChoice {
    index: u32,
    message: OpenAIMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAIUsage {
    #[serde(rename = "prompt_tokens")]
    prompt_tokens: Option<u32>,
    #[serde(rename = "completion_tokens")]
    completion_tokens: Option<u32>,
    /// Anthropic-style cache tokens — passed through by OpenRouter for Anthropic models.
    #[serde(default)]
    cache_creation_input_tokens: Option<u32>,
    #[serde(default)]
    cache_read_input_tokens: Option<u32>,
    /// OpenAI/DashScope-style cache hit reporting:
    /// `usage.prompt_tokens_details.cached_tokens`.
    #[serde(default)]
    prompt_tokens_details: Option<OpenAIPromptTokensDetails>,
    /// OpenAI/DashScope-style reasoning token reporting:
    /// `usage.completion_tokens_details.reasoning_tokens`.
    #[serde(default)]
    completion_tokens_details: Option<OpenAICompletionTokensDetails>,
}

impl OpenAIUsage {
    /// Effective cache-read tokens, merging the Anthropic field and the
    /// OpenAI/DashScope `prompt_tokens_details.cached_tokens` field.
    fn effective_cache_read(&self) -> u32 {
        self.cache_read_input_tokens
            .or_else(|| {
                self.prompt_tokens_details
                    .as_ref()
                    .and_then(|d| d.cached_tokens)
            })
            .unwrap_or(0)
    }

    /// Reasoning tokens from `completion_tokens_details.reasoning_tokens`.
    fn reasoning_tokens(&self) -> u32 {
        self.completion_tokens_details
            .as_ref()
            .and_then(|d| d.reasoning_tokens)
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
struct OpenAIPromptTokensDetails {
    #[serde(default)]
    cached_tokens: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct OpenAICompletionTokensDetails {
    #[serde(default)]
    reasoning_tokens: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct OpenAIStreamChunk {
    id: String,
    model: Option<String>,
    choices: Vec<OpenAIStreamChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<OpenAIUsage>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct OpenAIStreamChoice {
    index: u32,
    delta: Option<OpenAIMessageDelta>,
    message: Option<OpenAIMessageDelta>,
    finish_reason: Option<String>,
}

/// Streaming tool call — fields are optional because OpenAI sends them
/// incrementally: first chunk has id/type/name, continuation chunks only
/// have index + argument fragments.
#[derive(Debug, Clone, Deserialize)]
struct StreamingToolCall {
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<StreamingFunctionCall>,
}

#[derive(Debug, Clone, Deserialize)]
struct StreamingFunctionCall {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct OpenAIMessageDelta {
    role: Option<String>,
    content: Option<String>,
    #[serde(default, alias = "reasoning")]
    reasoning_content: Option<String>,
    tool_calls: Option<Vec<StreamingToolCall>>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAIErrorResponse {
    error: OpenAIError,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAIError {
    message: String,
    #[serde(rename = "type")]
    error_type: Option<String>,
}
