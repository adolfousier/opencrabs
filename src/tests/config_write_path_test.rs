//! Write-guard tests (#1199, #83).
//!
//! #1199: reads and writes disagreed about what a section is. The read side
//! resolves shorthand against a registry; the write side accepted any dotted
//! string and created whatever tables it named. An unknown section wrote
//! successfully into an orphan table that serde discards on load: tooling
//! reported success, the setting never applied, and reads kept honestly
//! returning the old value.
//!
//! #83: the guard is no longer a hand-maintained section list (two of those
//! drifted already — `CONFIG_SECTIONS` missed eight real sections and still
//! listed the migrated-away `[voice]`; `KNOWN_TOP_LEVEL_KEYS` missed
//! `doctor`). The compiled `Config` struct is the registry: a candidate
//! document is deserialized through it with `serde_ignored`, and a write is
//! valid iff the struct does not ignore a key at the target path.
//!
//! These tests exercise the `write_guard` unit directly against real `Config`
//! struct shapes; end-to-end behaviour through the actual write path (tool +
//! tempdir home) lives in `config_write_existing_section_test`.

use crate::config::sections::{known_sections, write_guard};

/// A candidate document containing only the requested section/key.
fn candidate(section: &str, key: &str, value: &str) -> String {
    format!("[{section}]\n{key} = {value}\n")
}

/// Unknown NEW top-level sections are hard-denied — a caller writing
/// `opencode` instead of `providers.opencode` gets a refusal and a hint, not
/// a silent orphan table (#1199, #83).
#[test]
fn unknown_new_sections_are_denied() {
    for (section, key, value) in [
        ("opencode", "base_url", "\"https://x\""),
        ("custom", "base_url", "\"https://x\""),
        ("inferhub", "api_key", "\"x\""),
        ("fallback", "enabled", "true"),
    ] {
        let err = write_guard(section, key, &candidate(section, key, value))
            .expect_err("unknown top-level section must be denied");
        assert!(
            err.contains("unknown config path") && err.contains(section),
            "denial must name the section: {err}"
        );
        assert!(err.contains("NOT changed"), "{err}");
    }
}

/// Every real top-level `Config` field accepts a write at the exact path —
/// the struct is the registry, so there is no hand-maintained list to drift.
#[test]
fn the_real_paths_are_accepted() {
    for (section, key, value) in [
        ("agent", "approval_policy", "\"auto\""),
        ("a2a", "enabled", "false"),
        ("channels.telegram.groups.mygroup", "name", "\"x\""),
        ("brain", "strip_empty_sections", "false"),
        ("browser", "cdp_endpoint", "\"\""),
        ("cron", "default_model", "\"x\""),
        ("database", "path", "\"/tmp/x.db\""),
        ("debug", "debug_lsp", "false"),
        ("image", "generation", "{ enabled = true, model = \"x\" }"),
        ("logging", "level", "\"info\""),
        ("providers.custom.myrepo", "base_url", "\"http://x\""),
        ("providers.fallback", "enabled", "false"),
        ("providers.opencode", "base_url", "\"http://x\""),
        ("provider_registry", "enabled", "false"),
    ] {
        let section_head = section.split('.').next().unwrap_or(section);
        assert!(
            known_sections().iter().any(|s| s == section_head),
            "test uses a real section: {section}"
        );
        write_guard(section, key, &candidate(section, key, value))
            .expect("real struct path must pass the guard");
    }
}

/// `memory` is a real section the guard accepts — the live incident that
/// started #83 (agents reached for raw file edits because the guard denied
/// `[memory].extra_paths`). A typo of it is denied, with the close match in
/// the error.
#[test]
fn memory_is_accepted_and_a_typo_of_memory_is_denied() {
    write_guard(
        "memory",
        "extra_paths",
        &candidate("memory", "extra_paths", "[\"/srv/docs\"]"),
    )
    .expect("memory is a real Config section");

    let err = write_guard(
        "memroy",
        "extra_paths",
        &candidate("memroy", "extra_paths", "[]"),
    )
    .expect_err("typo of memory must be denied");
    assert!(err.contains("memroy"), "{err}");
}

#[test]
fn empty_and_dot_padded_sections_are_denied() {
    assert!(write_guard("", "x", "true").unwrap_err().contains("empty"));
    assert!(
        write_guard("  providers.stt  ", "x", "true")
            .unwrap_err()
            .contains("whitespace")
    );
    assert!(
        write_guard("providers..stt", "x", "true")
            .unwrap_err()
            .contains("empty segment")
    );
}

/// The exact `section.key` path is judged, not just the section head: an
/// unknown key under a known section is as much a silent-drop risk as an
/// unknown section (#1199 at every depth).
#[test]
fn unknown_key_under_a_known_section_is_denied() {
    let err = write_guard(
        "agent",
        "bogus_leaf",
        &candidate("agent", "bogus_leaf", "true"),
    )
    .expect_err("unknown key under a known section must be denied");
    assert!(err.contains("agent.bogus_leaf"), "{err}");
}

/// Child shorthand is a READ convenience, not a writeable path — the guard
/// refuses a bare `[stt]` table and points at the real one.
#[test]
fn child_shorthand_in_the_section_is_denied_with_a_suggestion() {
    let err = write_guard("stt", "enabled", &candidate("stt", "enabled", "true"))
        .expect_err("child shorthand is not a writeable path");
    assert!(
        err.contains("providers.stt"),
        "suggests the parent path: {err}"
    );
}

/// The client-facing known-section list is struct-derived: every real
/// top-level section is present, and the two non-sections that used to leak
/// into hand-maintained lists are gone (`voice` is legacy — migrated into
/// `providers.stt/tts`; `gateway` is a serde alias of `a2a`).
#[test]
fn client_facing_read_list_matches_the_struct_derivation() {
    let known = known_sections();
    for real in [
        "agent",
        "daemon",
        "a2a",
        "image",
        "cron",
        "memory",
        "brain",
        "browser",
        "doctor",
        "channels",
        "provider_registry",
        "database",
        "logging",
        "debug",
        "providers",
        "tui",
    ] {
        assert!(
            known.iter().any(|s| s == real),
            "known_sections() must include {real}"
        );
    }
    assert!(
        !known.iter().any(|s| s == "voice"),
        "voice is legacy (migrated to providers) — it must not be a section"
    );
    assert!(
        !known.iter().any(|s| s == "gateway"),
        "gateway is a serde alias — the canonical field is a2a"
    );
}

#[test]
fn voice_is_not_a_writable_section() {
    // #1385: `voice` is a derived read-only view over providers.stt/tts;
    // writes must be rejected with a helpful suggestion.
    let err = write_guard(
        "voice",
        "tts_voice",
        &candidate("voice", "tts_voice", "\"x\""),
    )
    .expect_err("must reject voice writes");
    assert!(
        err.contains("providers.stt") && err.contains("providers.tts"),
        "must name the real tables, got: {err}"
    );
}

// ── pre-write convergence: legacy provider spelling must not deny writes ──

use crate::config::profile::with_home_override;

fn in_temp_home(f: impl FnOnce()) {
    let dir = tempfile::tempdir().expect("tempdir");
    let opencrabs = dir.path().join(".opencrabs");
    std::fs::create_dir_all(&opencrabs).expect("create .opencrabs");
    with_home_override(opencrabs, f);
}

#[test]
fn test_write_to_renamed_provider_section_succeeds_on_a_legacy_file() {
    // The zai rename shipped with config.toml.example still spelling the
    // section [providers.zhipu]. A wizard apply then wrote providers.zai.*
    // next to it, serde read the alias as a duplicate field, and the write
    // guard denied it — every onboarding ProviderAuth apply failed with
    // "providers.zai.enabled could not be saved". The writer must converge
    // the legacy spelling before inserting, exactly like the load path.
    in_temp_home(|| {
        let home = crate::config::opencrabs_home();
        std::fs::create_dir_all(&home).expect("home dir");
        let cfg = home.join("config.toml");
        std::fs::write(
            &cfg,
            "# my notes\n[providers.zhipu]\nenabled = false\ndefault_model = \"glm-5.1\"\n",
        )
        .expect("seed legacy config");

        crate::config::Config::write_key("providers.zai", "enabled", "true")
            .expect("write must converge the legacy section, not be denied");

        let after = std::fs::read_to_string(&cfg).unwrap();
        assert!(
            after.contains("[providers.zai]"),
            "canonical section present"
        );
        assert!(!after.contains("zhipu"), "legacy spelling converged away");
        assert!(after.contains("true"), "the written value landed");
        assert!(after.contains("# my notes"), "comments survive");
        // And the file still loads.
        crate::config::Config::load().expect("converged config parses");
    });
}
