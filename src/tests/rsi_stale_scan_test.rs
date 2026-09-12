//! Tests for the deterministic stale-claim scanner (#1240,
//! `src/brain/rsi_stale_scan.rs`).
//!
//! Coverage mirrors the RFC's verification list:
//! 1. historical-anchor exemption — dated lessons and violation ledgers
//!    mentioning dead tools must NEVER flag, even with imperative wording;
//! 2. typed verdicts everywhere — every verifier returns
//!    `Ok`/`Stale`/`Unverifiable`, and the undecided cases (globs, relative
//!    paths, map-valued config keys) return `Unverifiable`, never a false
//!    stale;
//! 3. amendment split — stale binaries/config keys/providers reword via
//!    `self_improve action='update'`; vanished paths queue for owner
//!    sign-off; NO action is ever a delete;
//! 4. schema membership against the compiled config types (never
//!    `config.toml.example`), pinned by the serde_ignored drift test (#83 —
//!    the compiled struct IS the registry, so there is no hand-maintained
//!    section list to drift).

use crate::brain::rsi_stale_scan::{
    ALL_ANCHOR_KINDS, AnchorKind, FindingAction, LineClass, Verdict, anchor_kind, backtick_spans,
    classify_line, decide_action, provider_mentions, scan_brain_files, verify_binary,
    verify_config_key, verify_path, verify_provider,
};
use crate::config::{Config, ProviderConfig};

// ---------------------------------------------------------------- exemption

/// RFC verification item 1, second half: dated-lesson mentions of dead
/// tools must NOT flag.
#[test]
fn historical_dated_lesson_is_exempt() {
    let line = "- `cmd-wrap` removed Aug 10 — use plain bash now";
    assert_eq!(classify_line(line), LineClass::HistoricalExempt);
}

#[test]
fn exemption_class_serializes_as_historical_exempt() {
    assert_eq!(
        serde_json::to_string(&LineClass::HistoricalExempt).unwrap(),
        "\"historical_exempt\""
    );
    assert_eq!(
        serde_json::to_string(&LineClass::Prescription).unwrap(),
        "\"prescription\""
    );
}

#[test]
fn violation_ledger_is_exempt() {
    let line = "Violations: 3, last 2026-08-22 (session fd72101f)";
    assert_eq!(classify_line(line), LineClass::HistoricalExempt);
}

/// Hint words alone ("ledger", "violation") carry the exemption even with
/// no date on the line.
#[test]
fn ledger_wording_without_date_is_exempt() {
    let line = "- Violation ledger: hit p95 twice (session f00d)";
    assert_eq!(classify_line(line), LineClass::HistoricalExempt);
}

/// Classifier boundary between the two scanned classes, made
/// unit-test-visible: identical imperative wording, one dated, one not.
/// The dated incident record wins even over imperative mood.
#[test]
fn imperative_mood_with_non_dated_refs_is_prescription_dated_counterpart_is_exempt() {
    let imperative = "- never use `cmd-wrap` in new sessions";
    assert_eq!(classify_line(imperative), LineClass::Prescription);

    let dated = "- never use `cmd-wrap` in new sessions (rule broken 2026-06-01)";
    assert_eq!(classify_line(dated), LineClass::HistoricalExempt);

    let prose = "The quick brown fox jumps over the lazy dog";
    assert_eq!(classify_line(prose), LineClass::Neutral);
}

#[test]
fn bare_year_marks_historical() {
    assert!(crate::brain::rsi_stale_scan::has_date_marker(
        "shipped in 2026"
    ));
    assert!(!crate::brain::rsi_stale_scan::has_date_marker(
        "no temporal marker here"
    ));
    // Month-fragment words must NOT count as dates (word boundaries).
    assert!(!crate::brain::rsi_stale_scan::has_date_marker(
        "the augment script"
    ));
    assert!(!crate::brain::rsi_stale_scan::has_date_marker(
        "decimate the old rows"
    ));
    assert!(!crate::brain::rsi_stale_scan::has_date_marker(
        "junior dev notes"
    ));
    assert!(crate::brain::rsi_stale_scan::has_date_marker(
        "removed on aug 12"
    ));
    assert!(crate::brain::rsi_stale_scan::has_date_marker(
        "2026-08-26: ledger entry"
    ));
    // Session-hex ids are not years (digit boundaries).
    assert!(!crate::brain::rsi_stale_scan::has_date_marker(
        "session fd72101f"
    ));
}

// ---------------------------------------------------------------- prescriptions

#[test]
fn imperative_rule_is_prescription() {
    let line = "- ALWAYS run `cargo clippy --all-features` before commit";
    assert_eq!(classify_line(line), LineClass::Prescription);
}

#[test]
fn dead_binary_in_prescription_verifies_stale() {
    let line = "- run `definitely-not-a-real-binary-9x2` before commit";
    assert_eq!(classify_line(line), LineClass::Prescription);
    assert_eq!(
        anchor_kind("definitely-not-a-real-binary-9x2", line),
        Some(AnchorKind::Binary)
    );
    assert_eq!(
        verify_binary("definitely-not-a-real-binary-9x2"),
        Verdict::Stale
    );
}

/// A span carrying argv (`cargo clippy --all-features`) verifies by its
/// FIRST token — checking the whole span against PATH would false-flag
/// every documented command line.
#[test]
fn binary_span_with_argv_verifies_first_token() {
    assert_eq!(verify_binary("sh -c 'echo hi'"), Verdict::Ok);
    assert_eq!(verify_binary("cargo clippy --all-features"), Verdict::Ok);
}

/// `cd` and friends are shell builtins: `which cd` finds nothing, but a
/// rule saying "use `cd`" is not stale.
#[test]
fn shell_builtin_verifies_ok() {
    for b in ["cd", "source", "export", "true"] {
        assert_eq!(verify_binary(b), Verdict::Ok, "builtin `{b}` must be Ok");
    }
}

// ---------------------------------------------------------------- amendment split

/// RFC design decision 2 as a pure boundary: autonomous sharpening versus
/// owner sign-off, over the full (kind × verdict) matrix.
#[test]
fn stale_findings_split_into_exactly_two_actions() {
    for kind in [
        AnchorKind::Binary,
        AnchorKind::ConfigKey,
        AnchorKind::ProviderName,
    ] {
        assert_eq!(
            decide_action(kind, Verdict::Stale),
            FindingAction::RewordViaUpdate,
            "stale {kind:?} must be rewordable via self_improve update"
        );
    }
    assert_eq!(
        decide_action(AnchorKind::FilePath, Verdict::Stale),
        FindingAction::SurfaceToUser,
        "vanished path is a removal candidate: never executed autonomously"
    );
    for kind in ALL_ANCHOR_KINDS {
        assert_eq!(decide_action(kind, Verdict::Ok), FindingAction::None);
        assert_eq!(
            decide_action(kind, Verdict::Unverifiable),
            FindingAction::None
        );
    }
}

/// The serde slugs pin the closed action vocabulary: there is no delete,
/// remove, or prune, and `None` serializes as "none".
#[test]
fn finding_action_slugs_pin_no_delete() {
    assert_eq!(
        serde_json::to_string(&FindingAction::RewordViaUpdate).unwrap(),
        "\"reword_via_update\""
    );
    assert_eq!(
        serde_json::to_string(&FindingAction::SurfaceToUser).unwrap(),
        "\"surface_to_user\""
    );
    assert_eq!(
        serde_json::to_string(&FindingAction::None).unwrap(),
        "\"none\""
    );
    // The exhaustive-match accessor agrees with the wire format — and its
    // no-wildcard match is what makes a future `Delete` variant a compile
    // error instead of a silent widening.
    for (a, slug) in [
        (FindingAction::RewordViaUpdate, "reword_via_update"),
        (FindingAction::SurfaceToUser, "surface_to_user"),
        (FindingAction::None, "none"),
    ] {
        assert_eq!(a.as_str(), slug);
        assert!(!slug.contains("delete") && !slug.contains("remove") && !slug.contains("prune"));
    }
    for kind in ALL_ANCHOR_KINDS {
        for verdict in [Verdict::Ok, Verdict::Stale, Verdict::Unverifiable] {
            let action = decide_action(kind, verdict);
            let slug = serde_json::to_string(&action).unwrap();
            assert!(
                !slug.contains("delete") && !slug.contains("remove") && !slug.contains("prune"),
                "action `{slug}` must never be a removal"
            );
        }
    }
}

// ---------------------------------------------------------------- anchor guards

/// Things that look like commands but are OpenCrabs-internal vocabulary.
#[test]
fn slash_command_is_not_world_state() {
    assert_eq!(anchor_kind("/help", "use `/help` when lost"), None);
}

#[test]
fn react_directive_is_skipped() {
    assert_eq!(anchor_kind("<<react:thumbsup>>", "emit <<react:👍>>"), None);
}

#[test]
fn snake_case_function_name_without_command_context_is_skipped() {
    let line = "the helper `scan_after_brain_write` runs post-write";
    assert_eq!(anchor_kind("scan_after_brain_write", line), None);
}

#[test]
fn file_path_missing_flags_with_surface_action() {
    let line = "- read `/definitely/missing/path-xyz.md` first";
    assert_eq!(
        anchor_kind("/definitely/missing/path-xyz.md", line),
        Some(AnchorKind::FilePath)
    );
    assert_eq!(
        decide_action(AnchorKind::FilePath, Verdict::Stale),
        FindingAction::SurfaceToUser
    );
}

// ---------------------------------------------------------------- path verdicts

#[test]
fn path_verdicts_are_typed_and_conservative() {
    // Glob patterns: existence of a pattern is not decidable.
    assert_eq!(verify_path("*.md"), Verdict::Unverifiable);
    // Relative and missing: may be repo-relative elsewhere — not evidence.
    assert_eq!(
        verify_path("no/such/relative-xyz.md"),
        Verdict::Unverifiable
    );
    // Absolute and missing: decidable stale.
    assert_eq!(
        verify_path("/definitely/no/such/file-9x.md"),
        Verdict::Stale
    );
    // Absolute and present.
    let tmp = std::env::temp_dir().join(format!("rsi_stale_scan_ok_{}", std::process::id()));
    std::fs::write(&tmp, "x").unwrap();
    assert_eq!(verify_path(tmp.to_str().unwrap()), Verdict::Ok);
    let _ = std::fs::remove_file(&tmp);
    // Missing under the real home (~ expands to an absolute path).
    assert_eq!(
        verify_path("~/definitely/no/such/thing-9x.md"),
        Verdict::Stale
    );
}

// ---------------------------------------------------------------- config keys

/// Membership runs against the compiled schema (serde structs), never
/// `config.toml.example` — RFC design decision 3.
#[test]
fn config_keys_verify_against_embedded_schema() {
    // Plain leaves and sections.
    assert_eq!(verify_config_key("[agent]"), Verdict::Ok);
    assert_eq!(verify_config_key("[agent.approval_policy]"), Verdict::Ok);
    assert_eq!(verify_config_key("[database.path]"), Verdict::Ok);
    assert_eq!(verify_config_key("[logging.level]"), Verdict::Ok);
    assert_eq!(verify_config_key("[doctor]"), Verdict::Ok);
    // `gateway` is a serde alias of `a2a`.
    assert_eq!(verify_config_key("[gateway]"), Verdict::Ok);
    // Option<struct> mid-path is patched into the witness.
    assert_eq!(verify_config_key("[memory.embedding.url]"), Verdict::Ok);
    // Provider leaves exist behind Option fields (sentinel exemplar).
    assert_eq!(
        verify_config_key("[providers.anthropic.api_key]"),
        Verdict::Ok
    );
    assert_eq!(
        verify_config_key("[providers.claude_cli.default_model]"),
        Verdict::Ok
    );
    assert_eq!(
        verify_config_key("[providers.fallback.enabled]"),
        Verdict::Ok
    );
    assert_eq!(
        verify_config_key("[providers.web_search.brave.api_key]"),
        Verdict::Ok
    );
    // Unknown sections and leaves — parents with visible field sets.
    assert_eq!(verify_config_key("[bogus_section_nope]"), Verdict::Stale);
    assert_eq!(verify_config_key("[agent.bogus_leaf]"), Verdict::Stale);
    assert_eq!(verify_config_key("[memory.bogus.leaf]"), Verdict::Stale);
    // Map-valued containers: user-chosen keys are not schema members, but
    // their absence proves nothing either.
    assert_eq!(
        verify_config_key("[channels.telegram.groups.mygroup.name]"),
        Verdict::Unverifiable
    );
    assert_eq!(
        verify_config_key("[providers.custom.myrepo.base_url]"),
        Verdict::Unverifiable
    );
    // Path continuing through a scalar leaf is malformed, not stale.
    assert_eq!(
        verify_config_key("[database.path.deeper]"),
        Verdict::Unverifiable
    );
}

/// Drift guard, struct-derived (#83): the same `serde_ignored` pass the
/// config write guard and the loader typo-warning use IS the registry — the
/// compiled `Config` struct — so there is no hand-maintained section list
/// left to pin (the old `KNOWN_TOP_LEVEL_KEYS` mirror blocked `doctor` and
/// went unnoticed precisely because the pinning test compared the witness to
/// its own hardcoded copy, never to the loader's list). These assertions pin
/// the pass itself:
/// - a section the struct knows (`[doctor]`, missing from the old list)
///   yields no ignored path;
/// - a typo (`[doctorr]`) yields exactly one, reported at its head;
/// - the struct-derived known-sections view includes every real top-level
///   section (and excludes the legacy `voice` and the `gateway` alias).
#[test]
fn schema_derived_sections_are_the_registry() {
    // Known section — including `doctor`, whose absence from the old
    // hand-maintained list caused a live false typo-warning on load.
    assert!(
        crate::config::sections::ignored_key_paths("[doctor]\nauto_fix = true\n")
            .unwrap()
            .is_empty(),
        "doctor is a real Config section — the struct must not ignore it"
    );

    // A typo of a known section is a top-level ignored path.
    assert_eq!(
        crate::config::sections::ignored_key_paths("[doctorr]\nauto_fix = true\n").unwrap(),
        vec!["doctorr".to_string()],
        "an unknown section is reported at its head"
    );

    // The struct-derived view covers every real section and only those.
    let known = crate::config::sections::known_sections();
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
        "tui",
        "channels",
        "provider_registry",
        "database",
        "logging",
        "debug",
        "providers",
    ] {
        assert!(
            known.iter().any(|s| s == real),
            "known_sections() must include {real}"
        );
    }
    assert!(!known.iter().any(|s| s == "voice"));
    assert!(!known.iter().any(|s| s == "gateway"));
}

// ---------------------------------------------------------------- providers

fn cfg_with(provider: &str) -> Config {
    let mut config = Config::default();
    let pc = Some(ProviderConfig {
        api_key: Some("sk-test".into()),
        ..Default::default()
    });
    let p = &mut config.providers;
    match provider {
        "anthropic" => p.anthropic = pc,
        "zai" => p.zai = pc,
        "github" => p.github = pc,
        _ => panic!("unknown provider in test helper"),
    }
    config
}

/// Provider verification checks THIS install's configured-provider table
/// (the same one `/models` uses), not the registry's theoretical universe.
#[test]
fn provider_verification_uses_configured_table() {
    let config = cfg_with("anthropic");
    assert_eq!(verify_provider("anthropic", &config), Verdict::Ok);
    // Aliases normalize to the configured id.
    assert_eq!(verify_provider("kimi", &cfg_with("zai")), Verdict::Stale);
    // The RFC's motivating example: a retired provider stays stale.
    assert_eq!(verify_provider("tencent/Hy3", &config), Verdict::Stale);
    // Known-but-unconfigured is stale guidance for THIS install.
    assert_eq!(verify_provider("openai", &config), Verdict::Stale);
    // CLI providers are configured by definition (no key needed) — they
    // surface via the binary anchor if the binary is gone.
    assert_eq!(verify_provider("claude-cli", &config), Verdict::Ok);
    assert_eq!(verify_provider("", &config), Verdict::Unverifiable);
}

/// The RFC's own dead-provider examples must flag stale when prescribed.
#[test]
fn retired_providers_flag_stale() {
    let config = Config::default();
    for dead in ["tencent/Hy3", "cmd-wrap", "omdc-proxy"] {
        assert_eq!(
            verify_provider(dead, &config),
            Verdict::Stale,
            "`{dead}` is retired and must verify stale"
        );
    }
}

#[test]
fn provider_mention_extraction_requires_cue_word() {
    let hits = provider_mentions("provider zhipu works; providers github too");
    assert_eq!(hits, vec!["github".to_string(), "zhipu".to_string()]);
    assert!(provider_mentions("the word banana does not count").is_empty());
}

#[test]
fn backtick_spans_preserve_order() {
    let spans = backtick_spans("use `foo` then `bar baz`");
    assert_eq!(spans, vec!["foo".to_string(), "bar baz".to_string()]);
}

// ---------------------------------------------------------------- end-to-end

fn scan_dir(
    lines: &str,
) -> (
    std::path::PathBuf,
    Vec<crate::brain::rsi_stale_scan::StaleFinding>,
) {
    // Unique per call: tests run in parallel threads of one process, so a
    // fixed pid-only name would let one test's remove_dir_all delete
    // another test's MEMORY.md mid-scan.
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rsi_stale_scan_e2e_{}_{}", std::process::id(), n));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("MEMORY.md"), lines).unwrap();
    let findings = scan_brain_files(&Config::default(), &dir);
    (dir, findings)
}

/// RFC verification item 1: dated lessons exempt, stale prescriptions split
/// by action class.
#[test]
fn scan_exempts_dated_lessons_but_splits_stale_prescriptions() {
    let (dir, findings) = scan_dir(concat!(
        // dated lesson mentioning a dead binary, COMMAND-SHAPED so it would
        // flag if the exemption were broken:
        "- run `cmd-wrap` (removed Aug 10) only when migrating old sessions\n",
        // imperative + dead binary → autonomous sharpen
        "- ALWAYS run `omdc-proxy-is-dead-7f3` before send\n",
        // imperative + vanished path → owner sign-off (removal candidate)
        "- read `/no/such/dir-abc123.md` first\n",
    ));
    let _ = std::fs::remove_dir_all(&dir);

    // Exemption: the dated lesson's dead binary never appears anywhere.
    assert!(
        !findings.iter().any(|f| f.anchor == "cmd-wrap"),
        "dated lesson must be exempt, got: {findings:?}"
    );
    // Split: exactly the two remaining anchors, one per action class.
    assert_eq!(findings.len(), 2, "unexpected findings: {findings:?}");
    let reword: Vec<_> = findings
        .iter()
        .filter(|f| f.action == FindingAction::RewordViaUpdate)
        .collect();
    let surface: Vec<_> = findings
        .iter()
        .filter(|f| f.action == FindingAction::SurfaceToUser)
        .collect();
    assert_eq!(
        reword.len(),
        1,
        "dead binary → reword_via_update: {findings:?}"
    );
    assert_eq!(reword[0].anchor, "omdc-proxy-is-dead-7f3");
    assert_eq!(
        surface.len(),
        1,
        "vanished path → surface_to_user: {findings:?}"
    );
    assert_eq!(surface[0].anchor, "/no/such/dir-abc123.md");
    // Ledger keys are stable and content-addressed.
    assert!(reword[0].unique_key().starts_with("MEMORY.md:2:binary:"));
    assert!(
        surface[0]
            .unique_key()
            .starts_with("MEMORY.md:3:file_path:")
    );
}

/// Ok and Unverifiable anchors are recorded with action None — they feed
/// the ledger's dedup (`last_verified`), never the amendment queue.
#[test]
fn scan_records_ok_and_unverifiable_with_no_action() {
    let (dir, findings) = scan_dir(concat!(
        "- run `sh -c 'echo hi'` to smoke the shell\n",
        "- read `*.md` files first\n",
        "- set `[agent.approval_policy]` to auto-session\n",
    ));
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(findings.len(), 3, "unexpected findings: {findings:?}");
    for f in &findings {
        assert_eq!(f.action, FindingAction::None, "nothing to amend: {f:?}");
        assert_ne!(f.verdict, Verdict::Stale);
    }
    let verdicts: Vec<(String, Verdict)> = findings
        .iter()
        .map(|f| (f.anchor.clone(), f.verdict))
        .collect();
    assert!(verdicts.contains(&("sh -c 'echo hi'".to_string(), Verdict::Ok)));
    assert!(verdicts.contains(&("*.md".to_string(), Verdict::Unverifiable)));
    assert!(verdicts.contains(&("[agent.approval_policy]".to_string(), Verdict::Ok)));
}

/// A prescribed provider that this install never configured flags stale
/// with the reword action.
#[test]
fn scan_flags_unconfigured_provider_mention() {
    let (dir, findings) = scan_dir("- route coding sessions to provider zhipu\n");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(findings.len(), 1, "unexpected findings: {findings:?}");
    assert_eq!(findings[0].anchor_kind, AnchorKind::ProviderName);
    assert_eq!(findings[0].verdict, Verdict::Stale);
    assert_eq!(findings[0].action, FindingAction::RewordViaUpdate);
}

/// A backticked provider name in provider context is a provider anchor,
/// not a binary.
#[test]
fn scan_treats_backticked_provider_name_as_provider() {
    let (dir, findings) = scan_dir("- always use provider `anthropic` for vision work\n");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(findings.len(), 1, "unexpected findings: {findings:?}");
    assert_eq!(findings[0].anchor_kind, AnchorKind::ProviderName);
    assert_eq!(findings[0].verdict, Verdict::Stale); // default config: no key
}
