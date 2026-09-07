//! Regression for the multi-profile cron daemon running its jobs toolless.
//!
//! The daemon built its own `ChannelFactory` but never wired a tool registry,
//! so every cron agent got an EMPTY registry and each tool call failed with
//! "Tool not found: bash". The fix routes both the interactive startup and the
//! daemon through `cli::tool_setup::register_core_agent_tools`. This pins that
//! the shared helper actually POPULATES the registry with the core tools cron
//! jobs use — catching an empty/regressed registry without spinning up a
//! daemon. (The daemon then wires the result via `factory.set_tool_registry`.)

use crate::brain::tools::registry::ToolRegistry;
use crate::cli::tool_setup::{register_core_agent_tools, register_runtime_tools};
use crate::config::Config;
use crate::db::Database;
use std::sync::Arc;

#[tokio::test]
async fn register_core_agent_tools_populates_registry_with_cron_tools() {
    let db = Database::connect_in_memory().await.expect("in-memory db");
    db.run_migrations().await.expect("migrations");
    let config = Config::default();

    let registry = Arc::new(ToolRegistry::new());
    let _subagent_manager = register_core_agent_tools(&registry, &db, &config, false);

    assert!(
        registry.count() > 0,
        "registry must not be empty — an empty registry is exactly the daemon bug"
    );

    // The concrete tools that failed with 'Tool not found' in the daemon log,
    // plus the rest of the core set cron jobs rely on.
    for name in [
        "bash",
        "read_file",
        "write_file",
        "edit_file",
        "ls",
        "glob",
        "grep",
        "execute_code",
        "session_search",
        "cron_manage",
        "plan",
        "web_search",
        "memory_search",
        "config_manager",
        "tool_search",
    ] {
        assert!(
            registry.has_tool(name),
            "core tool '{name}' must be registered by register_core_agent_tools"
        );
    }
}

/// The multi-profile cron daemon layers `register_runtime_tools` on top of the
/// core set so secondary-profile cron jobs match the primary profile's
/// functional tools (user-defined tools.toml tools, tool_manage, browser). This
/// pins that the runtime helper registers `tool_manage` (and, when the browser
/// feature is built, the browser tools) without touching live channel state.
#[tokio::test]
async fn register_runtime_tools_adds_dynamic_and_browser_tools() {
    let db = Database::connect_in_memory().await.expect("in-memory db");
    db.run_migrations().await.expect("migrations");
    let config = Config::default();

    let registry = Arc::new(ToolRegistry::new());
    let _subagent_manager = register_core_agent_tools(&registry, &db, &config, false);
    register_runtime_tools(&registry, &config);

    assert!(
        registry.has_tool("tool_manage"),
        "register_runtime_tools must register the dynamic-tool manager"
    );

    #[cfg(feature = "browser")]
    assert!(
        registry.has_tool("browser_navigate"),
        "register_runtime_tools must register browser tools when the feature is built"
    );

    // Channel-send tools need live in-process channel state the daemon never
    // builds — they must NOT leak into the runtime set (would be dead tools).
    assert!(
        !registry.has_tool("telegram_send"),
        "telegram_send must not be registered by the headless runtime helpers"
    );
}
