//! End-to-end write_config behaviour against a real config.toml in a
//! tempdir home (#83): the compiled `Config` struct is the registry.
//!
//! - sections the struct knows but the config tool only renders at the first
//!   level (`[memory]`, seeded into the home) write cleanly — the registry
//!   recognises them, no warning needed;
//! - genuinely unknown sections and keys are refused with the file left
//!   byte-identical;
//! - the post-write TOML parse guard still hard-denies schema-breaking
//!   writes (#714, untouched by #83).
//!
//! Tool-level tests use a sync poll of the `execute` future: the write path
//! never awaits, and a tokio runtime's task-local home override would not
//! survive into a freshly created runtime's task.

use std::path::{Path, PathBuf};
use std::task::{Context, Poll};

use crate::brain::tools::config_tool::ConfigTool;
use crate::brain::tools::{Tool, ToolExecutionContext, ToolResult};
use crate::config::Config;
use crate::config::profile::with_home_override;

/// A tempdir laid out as an `.opencrabs` home holding a seeded config.toml
/// whose `[memory]` section is real but unregistered by any config tool.
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

// ----------------------------------------------------------- write path --

/// The #83 live incident: `[memory]` is a real struct section that no config
/// tool renders — writing to it must pass with no warning and preserve the
/// existing keys.
#[test]
fn existing_unregistered_section_writes_without_a_warning() {
    let (_temp, home) = seeded_home();
    with_home_override(home.clone(), || {
        Config::write_key("memory", "extra_paths", "[\"/srv/docs\"]")
            .expect("struct-known section writes");
        let doc: toml_edit::DocumentMut = config_content(&home).parse().expect("valid TOML");
        assert_eq!(
            doc["memory"]["vector_enabled"].as_bool(),
            Some(false),
            "existing key preserved"
        );
        let arr = doc["memory"]["extra_paths"]
            .as_array()
            .expect("array written");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr.get(0).and_then(|v| v.as_str()), Some("/srv/docs"));
    });
}

/// A section the tool does render keeps working.
#[test]
fn registered_section_writes_cleanly() {
    let (_temp, home) = seeded_home();
    with_home_override(home.clone(), || {
        Config::write_key("agent", "approval_policy", "auto-always").expect("writes");
        let doc: toml_edit::DocumentMut = config_content(&home).parse().expect("valid TOML");
        assert_eq!(
            doc["agent"]["approval_policy"].as_str(),
            Some("auto-always")
        );
    });
}

#[test]
fn unknown_new_section_is_denied_and_the_file_untouched() {
    let (_temp, home) = seeded_home();
    with_home_override(home.clone(), || {
        let err = Config::write_key("opencode", "base_url", "https://x")
            .expect_err("unknown section must be denied");
        assert!(err.to_string().contains("opencode"), "{err}");
        assert_eq!(
            config_content(&home),
            "[memory]\nvector_enabled = false\n",
            "denied write must leave the file byte-identical"
        );
    });
}

#[test]
fn unknown_key_under_known_section_is_denied() {
    let (_temp, home) = seeded_home();
    with_home_override(home.clone(), || {
        let err = Config::write_key("agent", "bogus_leaf", "true")
            .expect_err("unknown key under a known section must be denied");
        assert!(err.to_string().contains("agent.bogus_leaf"), "{err}");
        assert_eq!(config_content(&home), "[memory]\nvector_enabled = false\n");
    });
}

/// The #714 parse guard still hard-denies a write that would break loading;
/// the schema guard runs after it, so this keeps the classic message.
#[test]
fn the_parse_guard_still_hard_denies_a_schema_breaking_write() {
    let (_temp, home) = seeded_home();
    with_home_override(home.clone(), || {
        let err = Config::write_key("memory", "vector_enabled", "not-a-bool")
            .expect_err("schema-breaking write must be denied");
        assert!(
            err.to_string().contains("would make config.toml invalid"),
            "{err}"
        );
        assert_eq!(config_content(&home), "[memory]\nvector_enabled = false\n");
    });
}

// -------------------------------------------------------- tool surface --

/// Poll an `execute` future to completion without a runtime (see module
/// doc: the write path is fully sync, and a real runtime task would lose
/// the tempdir home override).
fn execute_now(tool: &ConfigTool, input: serde_json::Value) -> ToolResult {
    let ctx = ToolExecutionContext::new(uuid::Uuid::new_v4());
    let mut fut = Box::pin(tool.execute(input, &ctx));
    let waker = futures::task::noop_waker();
    let mut cx = Context::from_waker(&waker);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(result) => result.expect("execute returns Ok"),
        Poll::Pending => panic!("config_manager execute must not await on this path"),
    }
}

#[test]
fn the_tool_accepts_a_known_unregistered_section() {
    let (_temp, home) = seeded_home();
    with_home_override(home.clone(), || {
        let result = execute_now(
            &ConfigTool,
            serde_json::json!({
                "operation": "write_config",
                "section": "memory",
                "key": "external_allowed_in_shared",
                "value": "true"
            }),
        );
        assert!(result.success, "{}", result.error.as_deref().unwrap_or(""));
        assert!(
            result
                .output
                .contains("Set [memory].external_allowed_in_shared = true in config.toml"),
            "{}",
            result.output
        );
        let doc: toml_edit::DocumentMut = config_content(&home).parse().expect("valid TOML");
        assert_eq!(
            doc["memory"]["external_allowed_in_shared"].as_bool(),
            Some(true)
        );
    });
}

#[test]
fn the_tool_still_refuses_an_unknown_new_section() {
    let (_temp, home) = seeded_home();
    with_home_override(home.clone(), || {
        let result = execute_now(
            &ConfigTool,
            serde_json::json!({
                "operation": "write_config",
                "section": "opencode",
                "key": "base_url",
                "value": "https://x"
            }),
        );
        assert!(!result.success);
        let msg = result.error.expect("error message");
        assert!(msg.contains("opencode"), "{msg}");
        assert_eq!(config_content(&home), "[memory]\nvector_enabled = false\n");
    });
}
