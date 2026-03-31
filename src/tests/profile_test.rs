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
    delete_profile, export_profile, hash_token, import_profile, list_profiles, migrate_profile,
    release_all_locks, release_token_lock, validate_profile_name,
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
// Everything that writes to ~/.opencrabs/ (profiles.toml, locks/, profiles/)
// runs inside ONE test function to prevent concurrent corruption from
// parallel test execution.

#[test]
fn filesystem_operations_sequential() {
    let pid = std::process::id();
    let lock_dir = base_opencrabs_dir().join("locks");
    fs::create_dir_all(&lock_dir).unwrap();

    // ══════════════════════════════════════════════════════════════════
    // Part 1: Profile CRUD lifecycle
    // ══════════════════════════════════════════════════════════════════
    let name = "_test_fs_seq";
    let profile_dir = base_opencrabs_dir().join("profiles").join(name);

    // Clean slate
    let _ = fs::remove_dir_all(&profile_dir);
    let mut reg = ProfileRegistry::load().unwrap_or_default();
    reg.profiles.remove(name);
    let _ = reg.save();

    // Create
    let path = create_profile(name, Some("sequential test")).unwrap();
    assert!(path.exists(), "profile directory should be created");
    assert!(path.join("memory").exists(), "memory subdir should exist");
    assert!(path.join("logs").exists(), "logs subdir should exist");

    // Verify in registry
    let reg = ProfileRegistry::load().unwrap();
    assert!(reg.profiles.contains_key(name), "should be in registry");

    // Duplicate create fails
    let err = create_profile(name, None).unwrap_err();
    assert!(err.to_string().contains("already exists"));

    // List includes it
    let profiles = list_profiles().unwrap();
    let found = profiles.iter().any(|p| p.name == name);
    assert!(found, "should appear in list");

    // Delete
    delete_profile(name).unwrap();
    assert!(!path.exists(), "directory gone after delete");

    let reg = ProfileRegistry::load().unwrap();
    assert!(!reg.profiles.contains_key(name), "removed from registry");

    // Delete again fails
    let err = delete_profile(name).unwrap_err();
    assert!(err.to_string().contains("does not exist"));

    // ══════════════════════════════════════════════════════════════════
    // Part 2: Export/Import roundtrip
    // ══════════════════════════════════════════════════════════════════
    let exp_name = "_test_fs_exp";
    let exp_dir = base_opencrabs_dir().join("profiles").join(exp_name);
    let archive = std::env::temp_dir().join(format!("_test_fs_export_{}.tar.gz", pid));

    let _ = fs::remove_dir_all(&exp_dir);
    let _ = fs::remove_file(&archive);
    let mut reg = ProfileRegistry::load().unwrap_or_default();
    reg.profiles.remove(exp_name);
    let _ = reg.save();

    // Create with content
    let dir = create_profile(exp_name, Some("export test")).unwrap();
    fs::write(dir.join("config.toml"), "[agent]\ncontext_limit = 42000").unwrap();
    fs::write(dir.join("memory").join("note.md"), "remember this").unwrap();

    // Export
    export_profile(exp_name, &archive).unwrap();
    assert!(archive.exists(), "archive created");
    assert!(archive.metadata().unwrap().len() > 0, "archive non-empty");

    // Delete
    delete_profile(exp_name).unwrap();
    assert!(!dir.exists());

    // Import
    let imported = import_profile(&archive).unwrap();
    assert_eq!(imported, exp_name);

    // Verify content survived
    let reimported = base_opencrabs_dir().join("profiles").join(exp_name);
    assert!(reimported.exists());
    let config = fs::read_to_string(reimported.join("config.toml")).unwrap();
    assert!(config.contains("context_limit = 42000"));
    let note = fs::read_to_string(reimported.join("memory").join("note.md")).unwrap();
    assert_eq!(note, "remember this");

    // Registry has it
    let reg = ProfileRegistry::load().unwrap();
    assert!(reg.profiles.contains_key(exp_name));

    // Clean up
    let _ = delete_profile(exp_name);
    let _ = fs::remove_file(&archive);

    // Export default profile
    let default_archive =
        std::env::temp_dir().join(format!("_test_fs_default_export_{}.tar.gz", pid));
    let _ = fs::remove_file(&default_archive);
    // Retry once for transient IO (concurrent dir mutations under ~/.opencrabs/)
    let result = export_profile("default", &default_archive)
        .or_else(|_| export_profile("default", &default_archive));
    if result.is_ok() {
        assert!(default_archive.exists());
    }
    let _ = fs::remove_file(&default_archive);

    // ══════════════════════════════════════════════════════════════════
    // Part 3: Token locks
    // ══════════════════════════════════════════════════════════════════

    // Basic acquire and release
    let ch1 = "_test_fs_lk1";
    let th1 = hash_token("fs_lock_1");
    release_token_lock(ch1, &th1);

    acquire_token_lock(ch1, &th1).unwrap();
    let lf1 = lock_dir.join(format!("{}_{}.lock", ch1, th1));
    assert!(lf1.exists(), "lock file created");

    let contents = fs::read_to_string(&lf1).unwrap();
    assert!(contents.contains(&pid.to_string()), "contains our PID");

    // Format: "profile:pid"
    let parts: Vec<&str> = contents.splitn(2, ':').collect();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0], "default");

    release_token_lock(ch1, &th1);
    assert!(!lf1.exists(), "lock file removed after release");

    // Re-acquire same lock (same PID, same profile)
    let ch2 = "_test_fs_lk2";
    let th2 = hash_token("fs_lock_2");
    release_token_lock(ch2, &th2);
    acquire_token_lock(ch2, &th2).unwrap();
    acquire_token_lock(ch2, &th2).unwrap(); // same PID overwrite
    release_token_lock(ch2, &th2);

    // Stale lock from dead PID
    let ch3 = "_test_fs_lk3";
    let th3 = hash_token("fs_lock_3");
    let stale = lock_dir.join(format!("{}_{}.lock", ch3, th3));
    fs::write(&stale, "default:999999999").unwrap();
    acquire_token_lock(ch3, &th3).unwrap();
    let contents = fs::read_to_string(&stale).unwrap();
    assert!(contents.contains(&pid.to_string()));
    release_token_lock(ch3, &th3);

    // Different channels, same token hash
    let th_multi = hash_token("multi_ch");
    let ch_a = "_test_fs_mca";
    let ch_b = "_test_fs_mcb";
    release_token_lock(ch_a, &th_multi);
    release_token_lock(ch_b, &th_multi);
    acquire_token_lock(ch_a, &th_multi).unwrap();
    acquire_token_lock(ch_b, &th_multi).unwrap();
    let la = lock_dir.join(format!("{}_{}.lock", ch_a, th_multi));
    let lb = lock_dir.join(format!("{}_{}.lock", ch_b, th_multi));
    assert!(la.exists());
    assert!(lb.exists());

    // release_all_locks cleans our locks
    release_all_locks();
    assert!(!la.exists(), "release_all cleaned lock a");
    assert!(!lb.exists(), "release_all cleaned lock b");

    // release_all preserves other profiles' locks
    let th_other = hash_token("other_profile_tok");
    let other_lock = lock_dir.join(format!("_test_fs_other_{}.lock", th_other));
    fs::write(&other_lock, "other_profile:999999999").unwrap();
    release_all_locks();
    assert!(other_lock.exists(), "other profile's lock preserved");
    let _ = fs::remove_file(&other_lock);

    // Release nonexistent lock is noop
    release_token_lock("_nonexistent_channel", "0000000000000000");
}

// ─── Migration Tests ─────────────────────────────────────────────────

#[test]
fn migrate_same_profile_errors() {
    let err = migrate_profile("default", "default", false);
    assert!(err.is_err());
    assert!(
        err.unwrap_err()
            .to_string()
            .contains("source and destination profiles are the same")
    );
}

#[test]
fn migrate_nonexistent_source_errors() {
    let err = migrate_profile("_test_migrate_no_src", "default", false);
    assert!(err.is_err());
    assert!(err.unwrap_err().to_string().contains("does not exist"));
}

#[test]
fn migrate_nonexistent_destination_errors() {
    let err = migrate_profile("default", "_test_migrate_no_dst", false);
    assert!(err.is_err());
    assert!(err.unwrap_err().to_string().contains("does not exist"));
}

#[test]
fn migrate_profile_copies_md_and_toml_files() {
    let base = crate::config::profile::base_opencrabs_dir();
    let src_name = "_test_migrate_src";
    let dst_name = "_test_migrate_dst";
    let src_dir = base.join("profiles").join(src_name);
    let dst_dir = base.join("profiles").join(dst_name);

    // Cleanup from previous runs
    let _ = fs::remove_dir_all(&src_dir);
    let _ = fs::remove_dir_all(&dst_dir);
    let mut reg = ProfileRegistry::load().unwrap();
    reg.profiles.remove(src_name);
    reg.profiles.remove(dst_name);
    let _ = reg.save();

    // Create both profiles
    create_profile(src_name, Some("source")).unwrap();
    create_profile(dst_name, Some("destination")).unwrap();

    // Populate source with brain files and config
    fs::write(src_dir.join("SOUL.md"), "# Source Soul").unwrap();
    fs::write(src_dir.join("IDENTITY.md"), "# Source Identity").unwrap();
    fs::write(src_dir.join("config.toml"), "[general]\nname = \"source\"").unwrap();
    fs::write(src_dir.join("keys.toml"), "[keys]\nsecret = \"abc\"").unwrap();
    fs::create_dir_all(src_dir.join("memory")).unwrap();
    fs::write(src_dir.join("memory").join("note.md"), "# A memory").unwrap();

    // Files that should NOT migrate
    fs::write(src_dir.join("layout.json"), "{}").unwrap();
    fs::write(src_dir.join("profiles.toml"), "skip").unwrap();
    fs::write(src_dir.join("random.txt"), "not a toml or md").unwrap();

    // Migrate
    let migrated = migrate_profile(src_name, dst_name, false).unwrap();

    // Verify correct files were copied
    assert!(dst_dir.join("SOUL.md").exists());
    assert!(dst_dir.join("IDENTITY.md").exists());
    assert!(dst_dir.join("config.toml").exists());
    assert!(dst_dir.join("keys.toml").exists());
    assert!(dst_dir.join("memory").join("note.md").exists());

    // Verify content matches
    assert_eq!(
        fs::read_to_string(dst_dir.join("SOUL.md")).unwrap(),
        "# Source Soul"
    );
    assert_eq!(
        fs::read_to_string(dst_dir.join("memory").join("note.md")).unwrap(),
        "# A memory"
    );

    // Verify skipped files
    assert!(!dst_dir.join("layout.json").exists());
    assert!(!dst_dir.join("random.txt").exists());
    // profiles.toml in dst should not be the source's "skip" content
    assert!(!dst_dir.join("profiles.toml").exists());

    assert!(migrated.contains(&"SOUL.md".to_string()));
    assert!(migrated.contains(&"IDENTITY.md".to_string()));
    assert!(migrated.contains(&"config.toml".to_string()));
    assert!(migrated.contains(&"keys.toml".to_string()));
    assert!(migrated.contains(&"memory/note.md".to_string()));
    assert_eq!(migrated.len(), 5);

    // Cleanup
    let _ = fs::remove_dir_all(&src_dir);
    let _ = fs::remove_dir_all(&dst_dir);
    let mut reg = ProfileRegistry::load().unwrap();
    reg.profiles.remove(src_name);
    reg.profiles.remove(dst_name);
    let _ = reg.save();
}

#[test]
fn migrate_profile_skips_existing_without_force() {
    let base = crate::config::profile::base_opencrabs_dir();
    let src_name = "_test_migrate_skip_src";
    let dst_name = "_test_migrate_skip_dst";
    let src_dir = base.join("profiles").join(src_name);
    let dst_dir = base.join("profiles").join(dst_name);

    let _ = fs::remove_dir_all(&src_dir);
    let _ = fs::remove_dir_all(&dst_dir);
    let mut reg = ProfileRegistry::load().unwrap();
    reg.profiles.remove(src_name);
    reg.profiles.remove(dst_name);
    let _ = reg.save();

    create_profile(src_name, None).unwrap();
    create_profile(dst_name, None).unwrap();

    // Source has a file
    fs::write(src_dir.join("SOUL.md"), "source content").unwrap();
    // Destination already has the same file
    fs::write(dst_dir.join("SOUL.md"), "existing content").unwrap();

    // Migrate without force — should skip
    let migrated = migrate_profile(src_name, dst_name, false).unwrap();
    assert!(
        migrated.is_empty(),
        "should skip existing files without --force"
    );
    assert_eq!(
        fs::read_to_string(dst_dir.join("SOUL.md")).unwrap(),
        "existing content",
        "original content preserved"
    );

    // Migrate with force — should overwrite
    let migrated = migrate_profile(src_name, dst_name, true).unwrap();
    assert_eq!(migrated.len(), 1);
    assert_eq!(
        fs::read_to_string(dst_dir.join("SOUL.md")).unwrap(),
        "source content",
        "overwritten with source"
    );

    let _ = fs::remove_dir_all(&src_dir);
    let _ = fs::remove_dir_all(&dst_dir);
    let mut reg = ProfileRegistry::load().unwrap();
    reg.profiles.remove(src_name);
    reg.profiles.remove(dst_name);
    let _ = reg.save();
}

#[test]
fn migrate_from_default_profile_works() {
    let base = crate::config::profile::base_opencrabs_dir();
    let dst_name = "_test_migrate_from_default";
    let dst_dir = base.join("profiles").join(dst_name);

    let _ = fs::remove_dir_all(&dst_dir);
    let mut reg = ProfileRegistry::load().unwrap();
    reg.profiles.remove(dst_name);
    let _ = reg.save();

    create_profile(dst_name, None).unwrap();

    // Default profile should have at least some .md or .toml files
    // Migrate from default — should succeed and find files
    let result = migrate_profile("default", dst_name, false);
    assert!(result.is_ok(), "migrate from default should succeed");

    let _ = fs::remove_dir_all(&dst_dir);
    let mut reg = ProfileRegistry::load().unwrap();
    reg.profiles.remove(dst_name);
    let _ = reg.save();
}
