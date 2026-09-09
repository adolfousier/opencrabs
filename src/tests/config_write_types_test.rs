//! End-to-end coverage for recursive `Config::write_key` value handling
//! (leshchenko1979/opencrabs#87): arrays-of-tables and objects survive as
//! real TOML structures, never silently dropped or stringified;
//! unrepresentable values (null, mixed-type arrays) are hard errors raised
//! BEFORE the file is touched, so config.toml keeps its exact bytes.
//!
//! Ported standalone for the upstream contribution: the fork's twin coverage
//! lives in `config_write_existing_section_test.rs` (fork-only write-guard
//! feature, shipped separately).

use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::config::profile::with_home_override;

/// A tempdir laid out as an `.opencrabs` home holding a seeded config.toml
/// with a real `[memory]` section.
fn seeded_home() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let opencrabs = dir.path().join(".opencrabs");
    std::fs::create_dir_all(&opencrabs).expect("create .opencrabs");
    std::fs::write(
        opencrabs.join("config.toml"),
        "[memory]\nvector_enabled = false\n",
    )
    .expect("write config");
    std::fs::write(opencrabs.join("keys.toml"), b"").expect("write keys");
    (dir, opencrabs)
}

fn config_content(home: &Path) -> String {
    std::fs::read_to_string(home.join("config.toml")).expect("config.toml readable")
}

/// The #87 incident case: `extra_paths` is an array-of-tables. The old
/// write_key dropped non-scalar array elements silently (the file ended up
/// with `extra_paths = []`); now each object survives as an inline table and
/// `Config::load()` sees the real path/pattern pair.
#[test]
fn array_of_tables_round_trips_the_87_incident_case() {
    let (_temp, home) = seeded_home();
    with_home_override(home.clone(), || {
        let written = Config::write_key(
            "memory",
            "extra_paths",
            r#"[{"path": "/root/opencrabs/src", "pattern": "**/*.rs"}]"#,
        )
        .expect("array-of-tables writes");
        assert!(
            written.contains("path = \"/root/opencrabs/src\""),
            "echo shows the object actually written: {written}"
        );

        let doc: toml_edit::DocumentMut = config_content(&home).parse().expect("valid TOML");
        let arr = doc["memory"]["extra_paths"]
            .as_array()
            .expect("array written");
        assert_eq!(arr.len(), 1);
        let entry = arr
            .get(0)
            .and_then(|v| v.as_inline_table())
            .expect("array-of-tables element is an inline table");
        assert_eq!(
            entry.get("path").and_then(|v| v.as_str()),
            Some("/root/opencrabs/src")
        );
        assert_eq!(
            entry.get("pattern").and_then(|v| v.as_str()),
            Some("**/*.rs")
        );

        let cfg = Config::load().expect("config still loads after the write");
        let paths = &cfg.memory.extra_paths;
        assert_eq!(paths.len(), 1);
        match &paths[0] {
            crate::config::ExtraPath::WithPattern { path, pattern } => {
                assert_eq!(path, "/root/opencrabs/src");
                assert_eq!(pattern, "**/*.rs");
            }
            other => panic!("expected WithPattern entry, got {other:?}"),
        }
    });
}

/// Object-shaped input becomes a real inline table, never a string literal
/// (the old `{...}` coerce-to-string behaviour, #87). Uses struct-known
/// fields (`url`, `model`) so the schema guard passes — `provider` is not an
/// `EmbeddingConfig` field.
#[test]
fn object_input_writes_an_inline_table() {
    let (_temp, home) = seeded_home();
    with_home_override(home.clone(), || {
        let written = Config::write_key(
            "memory",
            "embedding",
            r#"{"url": "https://api.openai.com/v1", "model": "text-embedding-3-small"}"#,
        )
        .expect("object writes");
        assert!(
            written.contains("model = \"text-embedding-3-small\""),
            "{written}"
        );

        let doc: toml_edit::DocumentMut = config_content(&home).parse().expect("valid TOML");
        let tbl = doc["memory"]["embedding"]
            .as_inline_table()
            .expect("object written as inline table");
        assert_eq!(
            tbl.get("url").and_then(|v| v.as_str()),
            Some("https://api.openai.com/v1")
        );
        assert_eq!(
            tbl.get("model").and_then(|v| v.as_str()),
            Some("text-embedding-3-small")
        );
    });
}

/// Scalar control: the plain integer path still lands as a real TOML integer.
#[test]
fn scalar_control_round_trips() {
    let (_temp, home) = seeded_home();
    with_home_override(home.clone(), || {
        let written =
            Config::write_key("memory", "sweep_interval_secs", "300").expect("integer writes");
        assert_eq!(written, "300");

        let doc: toml_edit::DocumentMut = config_content(&home).parse().expect("valid TOML");
        assert_eq!(doc["memory"]["sweep_interval_secs"].as_integer(), Some(300));
    });
}

/// `null` has no TOML representation — a hard error BEFORE the file is
/// touched: the conversion fails inside write_key, write_item never runs,
/// and config.toml keeps its exact bytes.
#[test]
fn null_value_is_a_hard_error_and_file_untouched() {
    let (_temp, home) = seeded_home();
    with_home_override(home.clone(), || {
        let err = Config::write_key("memory", "extra_paths", "[null]")
            .expect_err("null must be a hard error");
        assert!(!err.to_string().is_empty(), "error carries a message");
        assert!(err.to_string().contains("null"), "{}", err);
        assert_eq!(
            config_content(&home),
            "[memory]\nvector_enabled = false\n",
            "failed write must leave the file byte-identical"
        );
    });
}

/// TOML v1.0 arrays must be homogeneous; a JSON array mixing a number and a
/// string is unrepresentable — hard error, file untouched.
#[test]
fn mixed_type_array_is_a_hard_error_and_file_untouched() {
    let (_temp, home) = seeded_home();
    with_home_override(home.clone(), || {
        let err = Config::write_key("memory", "extra_paths", r#"[1, "x"]"#)
            .expect_err("mixed-type array must be a hard error");
        assert!(err.to_string().contains("mixes types"), "{}", err);
        assert_eq!(
            config_content(&home),
            "[memory]\nvector_enabled = false\n",
            "failed write must leave the file byte-identical"
        );
    });
}
