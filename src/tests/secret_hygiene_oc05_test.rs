//! Credential hygiene for config files and logs (OC-05).
//!
//! keys.toml holds provider API keys and channel tokens. It was written at the
//! process umask (0644), world-readable on a default home, so any local account
//! could read it. And the log-writer scrub matched only Telegram bot tokens, so
//! any other secret that rode a provider error's Display landed in the daily
//! log in plaintext. Both are closed here.

// ── keys.toml / config perms ────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn a_written_config_file_is_owner_only() {
    use crate::config::profile::with_home_override;
    use crate::config::{Config, opencrabs_home};
    use std::os::unix::fs::PermissionsExt;

    let home = tempfile::tempdir().expect("tempdir");
    with_home_override(home.path().to_path_buf(), || {
        Config::write_keys_key("providers.anthropic", "api_key", "sk-secret-value").expect("write");
        let mode = std::fs::metadata(opencrabs_home().join("keys.toml"))
            .expect("keys.toml exists")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o600,
            "keys.toml holds provider keys and channel tokens; it must not be group/world readable"
        );

        // config.toml funnels through the same atomic_write and is tightened too.
        Config::write_key("agent", "approval_policy", "\"auto-always\"").expect("write");
        let cfg_mode = std::fs::metadata(opencrabs_home().join("config.toml"))
            .expect("config.toml exists")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(cfg_mode, 0o600, "config.toml is written owner-only");
    });
}

// ── log scrub breadth ────────────────────────────────────────────────────────

#[test]
fn the_log_scrub_redacts_provider_keys_not_only_telegram_tokens() {
    use crate::logging::redact::scrub;

    // The bug: a non-Telegram secret in a log line survived. A realistic-length
    // OpenAI-style key must not appear in the scrubbed output.
    let line = "provider error: request failed with key sk-abcdefghijklmnopqrstuvwxyz012345";
    let out = scrub(line);
    assert!(
        !out.contains("sk-abcdefghijklmnopqrstuvwxyz012345"),
        "a provider key reached the log unredacted: {out}"
    );
    assert!(
        out.contains("[REDACTED]"),
        "the key should be replaced, got: {out}"
    );
}

#[test]
fn the_log_scrub_still_redacts_telegram_bot_tokens() {
    use crate::logging::redact::scrub;
    let line =
        "reqwest error at https://api.telegram.org/bot123456:AAAAAAAAAAAAAAAAAAAAAAAAA/sendMessage";
    let out = scrub(line);
    assert!(
        !out.contains("AAAAAAAAAAAAAAAAAAAAAAAAA"),
        "the bot token secret survived: {out}"
    );
}

#[test]
fn a_clean_log_line_is_returned_unchanged() {
    use crate::logging::redact::scrub;
    use std::borrow::Cow;
    let line = "session e4a1dacf started, 23 tools registered, model opus-5";
    match scrub(line) {
        Cow::Borrowed(s) => assert_eq!(s, line),
        Cow::Owned(s) => panic!("a line with no secret must borrow, not allocate: {s}"),
    }
}
