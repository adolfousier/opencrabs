use crate::utils::sanitize::*;
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
    // Construct at runtime so the literal token prefix never appears in source
    let token = String::from("xo") + "xb-" + "fake_test_token_not_real";
    let text = format!("slack token: {token}");
    let out = redact_secrets(&text);
    let expected = String::from("xo") + "xb-[REDACTED]";
    assert!(out.contains(&expected), "got: {out}");
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
    assert!(out.contains("SENDGRID_API_KEY=[REDACTED]"), "got: {out}");
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
    assert!(out.contains("DO_TOKEN=[REDACTED]"), "got: {out}");
}

// --- Unicode-expansion regression tests ---
// These verify that to_lowercase() byte-offset mismatch does not cause panic
// or wrong redaction when Unicode chars expand on lowercase (e.g. Turkish
// 'İ' → 'i̇' adds a combining dot, 2→3 bytes).

#[test]
fn redact_command_unicode_expansion_no_panic() {
    // 'İ' expands to 'i̇' (2→3 bytes) on to_lowercase().
    // Before the fix, match_pos exceeded result.len() and panicked.
    let input = "İİİİİİİİİİauthorization: bearer sk-secret-123";
    let out = redact_command(input);
    // Must not panic, and secret must be redacted
    assert!(out.contains("[REDACTED]"), "secret not redacted: {out}");
    assert!(!out.contains("sk-secret-123"), "secret leaked: {out}");
}

#[test]
fn redact_command_unicode_expansion_api_key() {
    // Same issue with api_key= prefix
    let input = "İİİİİİİİİİapi_key=super-secret-key";
    let out = redact_command(input);
    assert!(out.contains("[REDACTED]"), "secret not redacted: {out}");
    assert!(!out.contains("super-secret-key"), "secret leaked: {out}");
}

#[test]
fn redact_secrets_unicode_expansion_no_panic() {
    // Unicode expansion before an sk- key prefix — same panic scenario
    let input = "İİİİİİİİİİ sk-proj-mrRb3y9swLqHv8ZzB9lPH0_V7RPruzdbnXJf34DxU2RCdQnhCYjS99Tj";
    let out = redact_secrets(input);
    assert!(out.contains("[REDACTED]"), "secret not redacted: {out}");
    assert!(!out.contains("mrRb3y"), "secret leaked: {out}");
}

#[test]
fn redact_command_unicode_expansion_bearer() {
    // Unicode before "bearer " pattern
    let input = "İİİİİİİİİİ bearer eyJhbGc...";
    let out = redact_command(input);
    assert!(out.contains("[REDACTED]"), "token not redacted: {out}");
    assert!(!out.contains("eyJhbGc"), "token leaked: {out}");
}

#[test]
fn redact_secrets_unicode_expansion_bearer() {
    // Bearer pattern in redact_secrets with Unicode expansion
    let input = "İİİİİİİİİİbearer eyJhbGciOiJIUzI1NiJ9.test";
    let out = redact_secrets(input);
    assert!(out.contains("[REDACTED]"), "token not redacted: {out}");
    assert!(!out.contains("eyJhbGc"), "token leaked: {out}");
}

#[test]
fn redact_command_unicode_normal_text() {
    // Normal text with no secrets — should be unchanged
    let input = "Normal text with İstanbul and Größe and Ñoño";
    let out = redact_command(input);
    assert_eq!(out, input, "normal text should not change");
}

#[test]
fn redact_secrets_unicode_normal_text() {
    // Normal text with no secrets — should be unchanged
    let input = "Hello world, İstanbul, München, Ñoño";
    let out = redact_secrets(input);
    assert_eq!(out, input, "normal text should not change");
}

// --- Home path shortening tests ---

// Home-path shrinking tests construct inputs from `$HOME` and assert
// the redactor collapses them to `~`. On Windows there's no `HOME`
// by default — the redactor reads `%USERPROFILE%` (e.g.
// `C:\Users\runneradmin`) while these tests fall back to a fake
// `/Users/testuser` Unix path that the redactor never recognises.
// Gating the tests to Unix keeps the contract strict on the
// platforms where it actually applies.
#[cfg(unix)]
#[test]
fn shrinks_home_path_in_string() {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/testuser".to_string());
    let input = json!({"path": format!("{}/srv/rs/opencrabs", home)});
    let out = redact_tool_input(&input);
    assert_eq!(out["path"], "~/srv/rs/opencrabs");
}

#[cfg(unix)]
#[test]
fn shrinks_home_path_in_nested_object() {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/testuser".to_string());
    let input = json!({
        "config": {
            "dir": format!("{}/.opencrabs", home),
            "name": "test"
        }
    });
    let out = redact_tool_input(&input);
    assert_eq!(out["config"]["dir"], "~/.opencrabs");
    assert_eq!(out["config"]["name"], "test");
}

#[cfg(unix)]
#[test]
fn shrinks_home_path_in_array() {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/testuser".to_string());
    let input = json!([format!("{}/file1.rs", home), format!("{}/file2.rs", home)]);
    let out = redact_tool_input(&input);
    assert_eq!(out[0], "~/file1.rs");
    assert_eq!(out[1], "~/file2.rs");
}

#[cfg(unix)]
#[test]
fn shrinks_home_path_in_bash_command() {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/testuser".to_string());
    let input = json!({"command": format!("cat {}/.opencrabs/config.toml", home)});
    let out = redact_tool_input(&input);
    assert!(
        out["command"]
            .as_str()
            .unwrap()
            .contains("~/.opencrabs/config.toml")
    );
}

#[test]
fn preserves_non_home_paths() {
    let input = json!({"path": "/etc/hosts", "other": "/var/log/syslog"});
    let out = redact_tool_input(&input);
    assert_eq!(out["path"], "/etc/hosts");
    assert_eq!(out["other"], "/var/log/syslog");
}

#[cfg(unix)]
#[test]
fn shrinks_home_path_mid_string() {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/testuser".to_string());
    let input = json!({"msg": format!("Found at {}/docs/readme.md", home)});
    let out = redact_tool_input(&input);
    assert_eq!(out["msg"], "Found at ~/docs/readme.md");
}

#[test]
fn strip_think_tags_basic() {
    let input = "Hello <think>secret reasoning</think> world";
    assert_eq!(strip_think_tags(input), "Hello  world");
}

#[test]
fn strip_think_tags_multiple() {
    let input = "A<think>one<think>B<think>two</think>C";
    let out = strip_think_tags(input);
    assert!(!out.contains("<think>"), "tag leaked: {out}");
    assert!(!out.contains("one"), "first think leaked: {out}");
    assert!(out.contains('C'), "tail lost: {out}");
}

#[test]
fn strip_think_tags_unclosed_at_start_strips_the_rest() {
    // A stream cut mid-reasoning: the opener leads, so every byte after
    // it is reasoning and none of it is an answer.
    let input = "<think>this never closes and should all go";
    assert_eq!(strip_think_tags(input), "");
}

#[test]
fn strip_think_tags_unclosed_mid_text_keeps_the_answer() {
    // Regression: an opener that follows real text is prose mentioning
    // the tag, not reasoning. Deleting from there silently truncated
    // delivered messages — the sender saw a complete answer, the reader
    // got everything up to the mention and nothing after it.
    let input = "The gate gates on <think> and misses the longer form.";
    let out = strip_think_tags(input);
    assert!(!out.contains("<think>"), "tag leaked: {out}");
    assert!(
        out.contains("misses the longer form."),
        "text after the mention was eaten: {out}"
    );
}

#[test]
fn strip_thinking_tags_are_stripped() {
    // `find("<think>")` cannot match inside `<thinking>` (the byte after
    // `<think` is `i`, not `>`), so this form needs its own pass.
    let input = "Answer. <thinking>private deliberation</thinking> Done.";
    let out = strip_thinking_tags(input);
    assert!(!out.contains("<thinking>"), "tag leaked: {out}");
    assert!(
        !out.contains("private deliberation"),
        "reasoning leaked: {out}"
    );
    assert!(out.contains("Answer."), "answer lost: {out}");
    assert!(out.contains("Done."), "tail lost: {out}");
}

#[test]
fn strip_llm_artifacts_covers_the_longer_thinking_form() {
    // End-to-end through the delivery-side entry point: this is the shape
    // CLI providers serialize prior reasoning into the prompt as, which
    // teaches models to reproduce it as plain output text.
    let input = "<thinking>The user is asking about X.</thinking>\n\nHere is the answer.";
    let out = strip_llm_artifacts(input);
    assert!(!out.contains("<thinking>"), "tag leaked: {out}");
    assert!(
        !out.contains("The user is asking about X."),
        "reasoning leaked: {out}"
    );
    assert!(out.contains("Here is the answer."), "answer lost: {out}");
}

#[test]
fn strip_reasoning_tags_do_not_eat_the_next_character() {
    // The old copy advanced by 13 for a 12-byte close tag, swallowing the
    // first character after every block it stripped.
    let input = "<reasoning>internal</reasoning>Answer";
    assert_eq!(strip_reasoning_tags(input), "Answer");
}

#[test]
fn strip_think_tags_no_tags() {
    let input = "No thinking here";
    assert_eq!(strip_think_tags(input), "No thinking here");
}

#[test]
fn strip_reasoning_tags_basic() {
    // Two spaces, not one: removing the block leaves the space that
    // preceded the opener AND the space that followed the closer. This
    // asserted one space while the stripper advanced 13 bytes past a
    // 12-byte close tag, so the test was pinning the swallowed
    // character. `strip_think_tags_basic` above has always expected the
    // two-space form for the identical shape.
    let input = "Before <reasoning>internal</reasoning> After";
    assert_eq!(strip_reasoning_tags(input), "Before  After");
}

#[test]
fn strip_llm_artifacts_think_tags() {
    let input = "Response <think>hidden<think> more";
    let out = strip_llm_artifacts(input);
    assert!(!out.contains("<think>"), "think tag leaked: {out}");
    assert!(out.contains("Response"), "response lost: {out}");
}

// ─── strip_phantom_blocked (#1172) ──────────────────────────────────────────

#[test]
fn phantom_strip_removes_whole_flagged_section() {
    let input =
        "kept text\n<!-- phantom_blocked=1 -->\nnarration body\n<!-- /phantom_blocked=1 -->\nafter";
    let out = strip_phantom_blocked(input);
    assert!(!out.contains("narration body"), "body leaked: {out}");
    assert!(!out.contains("phantom_blocked"), "marker leaked: {out}");
    assert!(out.contains("kept text"), "prefix lost: {out}");
    assert!(out.contains("after"), "suffix lost: {out}");
}

#[test]
fn phantom_strip_handles_multiple_sections() {
    let input = "a <!-- phantom_blocked=1 -->x<!-- /phantom_blocked=1 --> b \
                 <!-- phantom_blocked=1 -->y<!-- /phantom_blocked=1 --> c";
    let out = strip_phantom_blocked(input);
    assert_eq!(out, "a  b  c");
}

#[test]
fn phantom_strip_unpaired_open_takes_tail() {
    // Truncated write: open without close — everything to the end is suspect.
    let input = "keep\n<!-- phantom_blocked=1 -->\ndangerous tail";
    let out = strip_phantom_blocked(input);
    assert_eq!(out, "keep\n");
}

#[test]
fn phantom_strip_passthrough_without_marker() {
    let input = "ordinary content with <!-- some other comment --> intact";
    assert_eq!(strip_phantom_blocked(input), input);
}

#[test]
fn strip_llm_artifacts_removes_ansi_csi_sequences() {
    // #1719: escape sequences in model/tool text get executed by the
    // terminal when the span buffer is drawn. They must not reach it.
    let input = "\u{1b}[31mred\u{1b}[0m plain";
    assert_eq!(strip_llm_artifacts(input), "red plain");
}

#[test]
fn strip_llm_artifacts_removes_osc_and_single_char_escapes() {
    let osc_bel = "\u{1b}]0;evil title\u{7}body";
    assert_eq!(strip_llm_artifacts(osc_bel), "body");
    let osc_st = "\u{1b}]2;t\u{1b}\\body";
    assert_eq!(strip_llm_artifacts(osc_st), "body");
    let single = "\u{1b}csave\u{1b}8rest";
    assert_eq!(strip_llm_artifacts(single), "saverest");
}

#[test]
fn strip_llm_artifacts_ansi_strip_keeps_plain_text_and_literal_brackets() {
    // No ESC byte anywhere: fast path, text byte-identical.
    let plain = "# Heading\n[31m not an escape\n`code [0m here`";
    assert_eq!(strip_llm_artifacts(plain), plain);
}

#[test]
fn strip_llm_artifacts_ansi_strip_survives_marker_then_escape_mix() {
    // ANSI pass runs before the Qwen marker sweep; both must apply.
    let input = "\u{1b}[2K<|tool\u{2581}call_begin|>hi";
    assert_eq!(strip_llm_artifacts(input), "hi");
}
