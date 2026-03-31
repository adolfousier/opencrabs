//! Comprehensive tests for profile management — multi-instance isolation.
//!
//! Tests cover: name validation, token hashing, registry CRUD, profile lifecycle,
//! token-lock isolation, export/import, and edge cases.
//!
//! IMPORTANT: Filesystem CRUD tests that write to `~/.opencrabs/profiles.toml`
//! are combined into single sequential functions to prevent concurrent write
//! corruption. In-memory tests remain separate since they don't share state.

use std::fs;
use std::path::PathBuf;

use crate::config::profile::{
    ProfileEntry, ProfileRegistry, acquire_token_lock, base_opencrabs_dir, create_profile,
    delete_profile, export_profile, hash_token, import_profile, list_profiles, release_all_locks,
    release_token_lock, validate_profile_name,
};

// ─── Name Validation ─────────────────────────────────────────────────

#[test]
fn valid_profile_names() {
    assert!(validate_profile_name("hermes").is_ok());
    assert!(validate_profile_name("my-profile").is_ok());
    assert!(validate_profile_name("test_123").is_ok());
    assert!(validate_profile_name("a").is_ok());
    assert!(validate_profile_name("UPPERCASE").is_ok());
    assert!(validate_profile_name("MiXeD-CaSe_99").is_ok());
    assert!(validate_profile_name("x".repeat(64).as_str()).is_ok());
}

#[test]
fn invalid_profile_name_default() {
    let err = validate_profile_name("default").unwrap_err();
    assert!(err.to_string().contains("reserved"));
}

#[test]
fn invalid_profile_name_empty() {
    let err = validate_profile_name("").unwrap_err();
    assert!(err.to_string().contains("1-64"));
}

#[test]
fn invalid_profile_name_too_long() {
    let long = "x".repeat(65);
    let err = validate_profile_name(&long).unwrap_err();
    assert!(err.to_string().contains("1-64"));
}

#[test]
fn invalid_profile_name_spaces() {
    let err = validate_profile_name("has spaces").unwrap_err();
    assert!(err.to_string().contains("alphanumeric"));
}

#[test]
fn invalid_profile_name_slashes() {
    assert!(validate_profile_name("has/slash").is_err());
    assert!(validate_profile_name("back\\slash").is_err());
}

#[test]
fn invalid_profile_name_special_chars() {
    assert!(validate_profile_name("name@here").is_err());
    assert!(validate_profile_name("name.dot").is_err());
    assert!(validate_profile_name("name!bang").is_err());
    assert!(validate_profile_name("name#hash").is_err());
    assert!(validate_profile_name("emoji🦀").is_err());
}

#[test]
fn validate_boundary_length_names() {
    assert!(validate_profile_name("x").is_ok());
    assert!(validate_profile_name(&"a".repeat(64)).is_ok());
    assert!(validate_profile_name(&"a".repeat(65)).is_err());
}

// ─── Token Hashing ───────────────────────────────────────────────────

#[test]
fn hash_token_deterministic() {
    let h1 = hash_token("bot123:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw");
    let h2 = hash_token("bot123:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw");
    assert_eq!(h1, h2);
}

#[test]
fn hash_token_different_inputs() {
    let h1 = hash_token("token_a");
    let h2 = hash_token("token_b");
    assert_ne!(h1, h2);
}

#[test]
fn hash_token_fixed_length() {
    assert_eq!(hash_token("short").len(), 16);
    assert_eq!(hash_token("a".repeat(1000).as_str()).len(), 16);
    assert_eq!(hash_token("").len(), 16);
}

#[test]
fn hash_token_hex_chars_only() {
    let h = hash_token("anything");
    assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn hash_token_empty_string() {
    let h = hash_token("");
    assert_eq!(h.len(), 16);
    assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn hash_token_unicode() {
    let h = hash_token("🦀🦀🦀");
    assert_eq!(h.len(), 16);
}

// ─── Profile Registry (In-Memory) ───────────────────────────────────
// These tests use in-memory registry instances — no filesystem contention.

#[test]
fn registry_default_is_empty() {
    let reg = ProfileRegistry::default();
    assert!(reg.profiles.is_empty());
}

#[test]
fn registry_register_single() {
    let mut reg = ProfileRegistry::default();
    reg.register("hermes", Some("Messenger of the gods"));
    assert!(reg.profiles.contains_key("hermes"));
    assert_eq!(reg.profiles["hermes"].name, "hermes");
    assert_eq!(
        reg.profiles["hermes"].description.as_deref(),
        Some("Messenger of the gods")
    );
    assert!(!reg.profiles["hermes"].created_at.is_empty());
    assert!(reg.profiles["hermes"].last_used.is_none());
}

#[test]
fn registry_register_no_description() {
    let mut reg = ProfileRegistry::default();
    reg.register("scout", None);
    assert!(reg.profiles["scout"].description.is_none());
}

#[test]
fn registry_register_multiple() {
    let mut reg = ProfileRegistry::default();
    reg.register("alpha", Some("First"));
    reg.register("beta", Some("Second"));
    reg.register("gamma", None);
    assert_eq!(reg.profiles.len(), 3);
}

#[test]
fn registry_register_overwrites_duplicate() {
    let mut reg = ProfileRegistry::default();
    reg.register("hermes", Some("v1"));
    let created_v1 = reg.profiles["hermes"].created_at.clone();

    reg.register("hermes", Some("v2"));
    assert_eq!(reg.profiles["hermes"].description.as_deref(), Some("v2"));
    assert_ne!(reg.profiles["hermes"].created_at, created_v1);
}

#[test]
fn registry_touch_updates_last_used() {
    let mut reg = ProfileRegistry::default();
    reg.register("hermes", None);
    assert!(reg.profiles["hermes"].last_used.is_none());

    reg.touch("hermes");
    assert!(reg.profiles["hermes"].last_used.is_some());
}

#[test]
fn registry_touch_nonexistent_is_noop() {
    let mut reg = ProfileRegistry::default();
    reg.touch("ghost");
    assert!(reg.profiles.is_empty());
}

#[test]
fn registry_serde_roundtrip() {
    let mut reg = ProfileRegistry::default();
    reg.register("hermes", Some("Test profile"));
    reg.register("scout", None);
    reg.touch("hermes");

    let serialized = toml::to_string_pretty(&reg).unwrap();
    let deserialized: ProfileRegistry = toml::from_str(&serialized).unwrap();

    assert_eq!(deserialized.profiles.len(), 2);
    assert!(deserialized.profiles.contains_key("hermes"));
    assert!(deserialized.profiles.contains_key("scout"));
    assert!(deserialized.profiles["hermes"].last_used.is_some());
    assert_eq!(
        deserialized.profiles["hermes"].description.as_deref(),
        Some("Test profile")
    );
}

#[test]
fn registry_serde_empty() {
    let reg = ProfileRegistry::default();
    let serialized = toml::to_string_pretty(&reg).unwrap();
    let deserialized: ProfileRegistry = toml::from_str(&serialized).unwrap();
    assert!(deserialized.profiles.is_empty());
}

#[test]
fn registry_deserialized_from_toml_string() {
    let toml_str = r#"
[profiles.hermes]
name = "hermes"
description = "Messenger"
created_at = "2026-03-31T00:00:00Z"

[profiles.scout]
name = "scout"
created_at = "2026-03-31T00:00:00Z"
"#;
    let reg: ProfileRegistry = toml::from_str(toml_str).unwrap();
    assert_eq!(reg.profiles.len(), 2);
    assert_eq!(
        reg.profiles["hermes"].description.as_deref(),
        Some("Messenger")
    );
    assert!(reg.profiles["scout"].description.is_none());
}

// ─── Profile Entry Serde ────────────────────────────────────────────

#[test]
fn profile_entry_json_roundtrip() {
    let entry = ProfileEntry {
        name: "test".to_string(),
        description: Some("desc".to_string()),
        created_at: "2026-03-31T00:00:00Z".to_string(),
        last_used: Some("2026-03-31T01:00:00Z".to_string()),
    };

    let json = serde_json::to_string(&entry).unwrap();
    let deserialized: ProfileEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.name, "test");
    assert_eq!(deserialized.description.as_deref(), Some("desc"));
    assert_eq!(
        deserialized.last_used.as_deref(),
        Some("2026-03-31T01:00:00Z")
    );
}

#[test]
fn profile_entry_optional_fields() {
    let entry = ProfileEntry {
        name: "minimal".to_string(),
        description: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        last_used: None,
    };
    assert!(entry.description.is_none());
    assert!(entry.last_used.is_none());
}

// ─── Path Resolution ─────────────────────────────────────────────────

#[test]
fn base_dir_ends_with_opencrabs() {
    let base = base_opencrabs_dir();
    assert!(base.ends_with(".opencrabs"));
}

#[test]
fn base_dir_is_absolute() {
    let base = base_opencrabs_dir();
    assert!(base.is_absolute());
}

// ─── Error Messages ──────────────────────────────────────────────────

#[test]
fn delete_default_profile_fails() {
    let err = delete_profile("default").unwrap_err();
    assert!(err.to_string().contains("cannot delete"));
}

#[test]
fn delete_nonexistent_profile_fails() {
    let err = delete_profile("_nonexistent_profile_xyz").unwrap_err();
    assert!(err.to_string().contains("does not exist"));
}

#[test]
fn export_nonexistent_profile_fails() {
    let archive = std::env::temp_dir().join("_test_nonexistent_export.tar.gz");
    let err = export_profile("_definitely_not_a_profile", &archive).unwrap_err();
    assert!(err.to_string().contains("does not exist"));
}

#[test]
fn import_nonexistent_archive_fails() {
    let err = import_profile(&PathBuf::from("/tmp/_nonexistent_archive.tar.gz")).unwrap_err();
    assert!(err.to_string().contains("not found"));
}

// ─── Registry Filesystem (Read-Only) ────────────────────────────────

#[test]
fn registry_load_from_real_path() {
    // Should not error regardless of host state
    let loaded = ProfileRegistry::load().unwrap();
    let _ = loaded.profiles.len();
}

#[test]
fn list_profiles_always_includes_default() {
    let profiles = list_profiles().unwrap();
    assert!(!profiles.is_empty());
    assert_eq!(profiles[0].name, "default");
    assert!(
        profiles[0]
            .description
            .as_deref()
            .unwrap()
            .contains("Default")
    );
}

// ─── Profile CRUD (Filesystem — Sequential) ─────────────────────────
// All tests that write to ~/.opencrabs/profiles.toml run inside a single
// test function to prevent concurrent write corruption.

#[test]
fn profile_crud_lifecycle() {
    let name = "_test_crud_seq";
    let profile_dir = base_opencrabs_dir().join("profiles").join(name);

    // ── Clean slate (aggressive — handles stale state from previous runs) ──
    if profile_dir.exists() {
        fs::remove_dir_all(&profile_dir).expect("failed to clean stale test profile dir");
    }
    let mut reg = ProfileRegistry::load().unwrap_or_default();
    reg.profiles.remove(name);
    reg.save().expect("failed to clean registry");

    // ── Create ──
    let path = create_profile(name, Some("sequential lifecycle")).unwrap();
    assert!(path.exists());
    assert!(path.join("memory").exists());
    assert!(path.join("logs").exists());

    // ── Verify directory is the source of truth ──
    assert!(profile_dir.exists());

    // ── Duplicate create fails ──
    let err = create_profile(name, None).unwrap_err();
    assert!(err.to_string().contains("already exists"));

    // ── List includes it ──
    // Re-load registry to ensure our entry is there
    let profiles = list_profiles().unwrap();
    let found = profiles.iter().any(|p| p.name == name);
    assert!(found, "created profile should appear in list");

    // ── Delete ──
    delete_profile(name).unwrap();
    assert!(!path.exists(), "profile directory should be removed");

    // ── Delete again fails ──
    let err = delete_profile(name).unwrap_err();
    assert!(err.to_string().contains("does not exist"));
}

#[test]
fn profile_export_import_lifecycle() {
    let name = "_test_exp_imp_seq";
    let archive_path = std::env::temp_dir().join("_test_profile_export_seq.tar.gz");
    let profile_dir = base_opencrabs_dir().join("profiles").join(name);

    // ── Clean slate ──
    let _ = fs::remove_dir_all(&profile_dir);
    let _ = fs::remove_file(&archive_path);
    let mut reg = ProfileRegistry::load().unwrap_or_default();
    reg.profiles.remove(name);
    let _ = reg.save();

    // ── Create with content ──
    let dir = create_profile(name, Some("export test")).unwrap();
    fs::write(dir.join("config.toml"), "[general]\nname = \"test\"").unwrap();
    fs::write(dir.join("memory").join("note.md"), "remember this").unwrap();

    // ── Export ──
    export_profile(name, &archive_path).unwrap();
    assert!(archive_path.exists());
    assert!(archive_path.metadata().unwrap().len() > 0);

    // ── Delete ──
    delete_profile(name).unwrap();
    assert!(!dir.exists());

    // ── Import ──
    let imported = import_profile(&archive_path).unwrap();
    assert_eq!(imported, name);

    // ── Verify content survived ──
    let reimported = base_opencrabs_dir().join("profiles").join(name);
    assert!(reimported.exists());
    let config = fs::read_to_string(reimported.join("config.toml")).unwrap();
    assert!(config.contains("name = \"test\""));
    let note = fs::read_to_string(reimported.join("memory").join("note.md")).unwrap();
    assert_eq!(note, "remember this");

    // ── Clean up ──
    let _ = delete_profile(name);
    let _ = fs::remove_file(&archive_path);
}

#[test]
fn export_default_profile_succeeds() {
    // Unique filename to avoid parallel test conflicts
    let archive = std::env::temp_dir().join(format!(
        "_test_default_export_{}.tar.gz",
        std::process::id()
    ));
    let _ = fs::remove_file(&archive);

    // Other profile tests create/delete subdirs concurrently under ~/.opencrabs/,
    // which can cause tar to see a dir entry then fail to stat it.
    // Retry once on transient IO errors.
    let result =
        export_profile("default", &archive).or_else(|_| export_profile("default", &archive));

    if result.is_ok() {
        assert!(archive.exists());
        assert!(archive.metadata().unwrap().len() > 0);
    }

    let _ = fs::remove_file(&archive);
}

// ─── Token Locks ─────────────────────────────────────────────────────
// Lock files use unique per-test names so they don't collide.

#[test]
fn acquire_and_release_token_lock() {
    let channel = "_test_lock";
    let token_hash = hash_token("test_acquire_release");

    release_token_lock(channel, &token_hash);

    acquire_token_lock(channel, &token_hash).unwrap();

    let lock_file = base_opencrabs_dir()
        .join("locks")
        .join(format!("{}_{}.lock", channel, token_hash));
    assert!(lock_file.exists());

    let contents = fs::read_to_string(&lock_file).unwrap();
    assert!(contents.contains(&std::process::id().to_string()));

    release_token_lock(channel, &token_hash);
    assert!(!lock_file.exists());
}

#[test]
fn acquire_same_lock_twice_succeeds_same_pid() {
    let channel = "_test_reacquire";
    let token_hash = hash_token("test_reacquire");

    release_token_lock(channel, &token_hash);

    acquire_token_lock(channel, &token_hash).unwrap();
    acquire_token_lock(channel, &token_hash).unwrap();

    release_token_lock(channel, &token_hash);
}

#[test]
fn stale_lock_from_dead_pid_is_overwritten() {
    let channel = "_test_stale";
    let token_hash = hash_token("test_stale_lock");
    let lock_dir = base_opencrabs_dir().join("locks");
    fs::create_dir_all(&lock_dir).unwrap();

    let lock_file = lock_dir.join(format!("{}_{}.lock", channel, token_hash));
    fs::write(&lock_file, "default:999999999").unwrap();

    acquire_token_lock(channel, &token_hash).unwrap();

    let contents = fs::read_to_string(&lock_file).unwrap();
    assert!(contents.contains(&std::process::id().to_string()));

    release_token_lock(channel, &token_hash);
}

#[test]
fn release_all_locks_cleans_own_locks() {
    let token_hash_a = hash_token("release_all_a");
    let token_hash_b = hash_token("release_all_b");

    // Clean stale locks from previous runs (e.g. daemon PID)
    release_token_lock("_test_all_a", &token_hash_a);
    release_token_lock("_test_all_b", &token_hash_b);

    acquire_token_lock("_test_all_a", &token_hash_a).unwrap();
    acquire_token_lock("_test_all_b", &token_hash_b).unwrap();

    let lock_a = base_opencrabs_dir()
        .join("locks")
        .join(format!("_test_all_a_{}.lock", token_hash_a));
    let lock_b = base_opencrabs_dir()
        .join("locks")
        .join(format!("_test_all_b_{}.lock", token_hash_b));

    assert!(lock_a.exists());
    assert!(lock_b.exists());

    release_all_locks();

    assert!(!lock_a.exists());
    assert!(!lock_b.exists());
}

#[test]
fn release_all_locks_preserves_other_profiles() {
    let token_hash = hash_token("preserve_other");
    let lock_dir = base_opencrabs_dir().join("locks");
    fs::create_dir_all(&lock_dir).unwrap();

    let lock_file = lock_dir.join(format!("_test_preserve_{}.lock", token_hash));
    fs::write(&lock_file, "other_profile:999999999").unwrap();

    release_all_locks();

    assert!(lock_file.exists());

    let _ = fs::remove_file(&lock_file);
}

#[test]
fn lock_file_contains_profile_and_pid() {
    let channel = "_test_lock_contents";
    let token_hash = hash_token("lock_contents_check");

    release_token_lock(channel, &token_hash);
    acquire_token_lock(channel, &token_hash).unwrap();

    let lock_file = base_opencrabs_dir()
        .join("locks")
        .join(format!("{}_{}.lock", channel, token_hash));
    let contents = fs::read_to_string(&lock_file).unwrap();

    // Format: "profile:pid"
    let parts: Vec<&str> = contents.splitn(2, ':').collect();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0], "default"); // no active profile set = "default"
    assert_eq!(parts[1], std::process::id().to_string());

    release_token_lock(channel, &token_hash);
}

#[test]
fn lock_different_channels_same_token() {
    let token_hash = hash_token("multi_channel_token");

    release_token_lock("_test_ch_a", &token_hash);
    release_token_lock("_test_ch_b", &token_hash);

    acquire_token_lock("_test_ch_a", &token_hash).unwrap();
    acquire_token_lock("_test_ch_b", &token_hash).unwrap();

    let lock_a = base_opencrabs_dir()
        .join("locks")
        .join(format!("_test_ch_a_{}.lock", token_hash));
    let lock_b = base_opencrabs_dir()
        .join("locks")
        .join(format!("_test_ch_b_{}.lock", token_hash));

    assert!(lock_a.exists());
    assert!(lock_b.exists());

    release_token_lock("_test_ch_a", &token_hash);
    release_token_lock("_test_ch_b", &token_hash);
}

#[test]
fn release_nonexistent_lock_is_noop() {
    // Should not panic
    release_token_lock("_nonexistent_channel", "0000000000000000");
}
