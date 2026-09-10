//! Every execution tool honors the bash hard blocklist (OC-06).
//!
//! `bash` documents `check_blocked_command` as the immovable floor that applies
//! even under approval, but `execute_code`, dynamic shell executors, and cron
//! ran their commands without it, so `rm -rf ~` (and fork bombs, /etc/shadow
//! reads, …) through those tools was a complete bypass. `assert_command_allowed`
//! is the single floor they now share.

use crate::brain::tools::bash::assert_command_allowed;

#[test]
fn the_floor_refuses_a_destructive_command() {
    // Reuse the shapes already in the bash blocklist rather than inventing new
    // destructive strings.
    assert!(assert_command_allowed("rm -rf ~").is_some());
    assert!(assert_command_allowed("rm -rf /").is_some());
}

#[test]
fn the_floor_allows_an_ordinary_command() {
    assert!(assert_command_allowed("echo hello").is_none());
    assert!(assert_command_allowed("ls -la /tmp").is_none());
}

// ── the other execution tools actually call the floor ────────────────────────

const EXEC_TOOLS: &[&str] = &[
    "src/brain/tools/code_exec.rs",
    "src/brain/tools/dynamic/tool.rs",
];

#[test]
fn every_execution_tool_calls_the_shared_floor() {
    for rel in EXEC_TOOLS {
        let text = std::fs::read_to_string(std::path::Path::new(rel))
            .unwrap_or_else(|e| panic!("{rel} must be readable ({e})"));
        assert!(
            text.contains("assert_command_allowed"),
            "{rel} runs commands but does not call assert_command_allowed, so it can bypass the \
             bash blocklist (OC-06)"
        );
    }
}

#[test]
fn a_dynamic_tool_cannot_reserve_a_core_tool_name() {
    // The loader must refuse a dynamic tool that would shadow a core one.
    let loader = std::fs::read_to_string("src/brain/tools/dynamic/loader.rs").unwrap();
    assert!(
        loader.contains("reserved core tool name") && loader.contains("\"bash\""),
        "add_tool must reserve core tool names so a dynamic 'bash' cannot replace the real one"
    );
}
