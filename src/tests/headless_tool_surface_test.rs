//! #129 — headless tool-surface honesty (owner-ruled design, issue body).
//!
//! Pins the three layers of the headless gate:
//! (a) the headless registry lacks `session_notify` + `suggest_options`,
//! (b) the interactive registry keeps them,
//! (c) both tools hard-error when invoked with a headless context
//!     (belt-and-braces backstop),
//! (d) the headless preamble is a standalone const and the registry gate
//!     documentation matches the owner ruling C (sub-agents always headless:
//!     both tools in ALWAYS_EXCLUDED).

use crate::brain::tools::Tool;
use crate::brain::tools::error::ToolError;
use crate::brain::tools::registry::ToolRegistry;
use crate::brain::tools::subagent::ALWAYS_EXCLUDED;
use crate::brain::tools::r#trait::ToolExecutionContext;
use crate::brain::tools::{subagent, suggest_options::SuggestOptionsTool};
use crate::cli::tool_setup::{HEADLESS_PREAMBLE, register_core_agent_tools};
use crate::config::Config;
use crate::db::Database;
use serde_json::json;
use std::sync::Arc;

async fn build_registry(headless: bool) -> (Arc<ToolRegistry>, ()) {
    let db = Database::connect_in_memory().await.expect("in-memory db");
    db.run_migrations().await.expect("migrations");
    let config = Config::default();
    let registry = Arc::new(ToolRegistry::new());
    let _m = register_core_agent_tools(&registry, &db, &config, headless);
    (registry, ())
}

/// (a) Headless registry: both interactive-only tools absent.
#[tokio::test]
async fn headless_registry_lacks_session_notify_and_suggest_options() {
    let (registry, _) = build_registry(true).await;
    assert!(
        !registry.has_tool("session_notify"),
        "session_notify must NOT be registered headless (#129): a one-shot \
         process cannot flush a parked notification"
    );
    assert!(
        !registry.has_tool("suggest_options"),
        "suggest_options must NOT be registered headless (#129): it renders \
         nowhere without a TUI/channel handler"
    );
    // Sanity: the core set is still intact on the headless path.
    assert!(
        registry.has_tool("bash") && registry.has_tool("read_file"),
        "core tools must remain registered headless"
    );
}

/// (b) Interactive registry: both tools present.
#[tokio::test]
async fn interactive_registry_keeps_session_notify_and_suggest_options() {
    let (registry, _) = build_registry(false).await;
    assert!(
        registry.has_tool("session_notify"),
        "interactive registry must keep session_notify"
    );
    assert!(
        registry.has_tool("suggest_options"),
        "interactive registry must keep suggest_options"
    );
}

/// (c) Belt-and-braces: both tools hard-error on a headless context.
#[tokio::test]
async fn interactive_only_tools_hard_error_on_headless_context() {
    let mut ctx = ToolExecutionContext::new(uuid::Uuid::new_v4());
    ctx.headless = true;

    let suggest_err = <SuggestOptionsTool as Tool>::execute(
        SuggestOptionsTool,
        json!({ "options": ["Go", "Stop"] }),
        &ctx,
    )
    .await
    .expect_err("suggest_options must hard-error headless (#129)");
    assert!(
        matches!(suggest_err, ToolError::Execution(ref m) if m.contains("not available headless")),
        "suggest_options headless error must be a loud ToolError, got: {suggest_err:?}"
    );

    let notify_err = <subagent::SessionNotifyTool as Tool>::execute(
        subagent::SessionNotifyTool,
        json!({ "target_session": uuid::Uuid::new_v4().to_string(), "text": "hi" }),
        &ctx,
    )
    .await
    .expect_err("session_notify must hard-error headless (#129)");
    assert!(
        matches!(notify_err, ToolError::Execution(ref m) if m.contains("not available headless")),
        "session_notify headless error must be a loud ToolError, got: {notify_err:?}"
    );
}

/// (c, cont.) Interactive context: no guard trip — suggest_options proceeds
/// past the headless check (it fails later on missing progress sink or
/// returns a normal verdict, but NEVER with the headless error).
#[tokio::test]
async fn suggest_options_does_not_trip_guard_on_interactive_context() {
    let ctx = ToolExecutionContext::new(uuid::Uuid::new_v4());
    let result = <SuggestOptionsTool as Tool>::execute(
        SuggestOptionsTool,
        json!({ "options": ["Go"] }),
        &ctx,
    )
    .await;
    match result {
        Err(ToolError::Execution(m)) if m.contains("not available headless") => {
            panic!("headless guard must not fire on an interactive context")
        }
        _ => {} // normal path (verdict or unrelated error) is fine
    }
}

/// (d) Preamble content + sub-agent surface law.
#[test]
fn headless_preamble_names_the_final_message_law() {
    assert!(
        HEADLESS_PREAMBLE.contains("HEADLESS"),
        "preamble must identify the surface"
    );
    assert!(
        HEADLESS_PREAMBLE.to_lowercase().contains("final message"),
        "preamble must carry the final-message self-containedness law"
    );
    // AGENTS.md was deliberately NOT touched (owner design): the preamble is
    // the ONLY vehicle for the headless-scoped rule.
    assert!(
        HEADLESS_PREAMBLE.contains("session_notify")
            && HEADLESS_PREAMBLE.contains("suggest_options"),
        "preamble must warn the model off the two unavailable tools"
    );
}

/// Owner ruling C: sub-agents are ALWAYS headless — both tools sit in
/// ALWAYS_EXCLUDED, so build_child_registry strips them even from an
/// interactive parent's registry.
#[test]
fn child_registry_strips_interactive_only_tools() {
    assert!(
        ALWAYS_EXCLUDED.contains(&"session_notify"),
        "session_notify must be in ALWAYS_EXCLUDED (ruling C)"
    );
    assert!(
        ALWAYS_EXCLUDED.contains(&"suggest_options"),
        "suggest_options must be in ALWAYS_EXCLUDED (ruling C)"
    );

    // Simulate an interactive parent registry carrying both tools.
    let parent = ToolRegistry::new();
    parent.register(Arc::new(subagent::SessionNotifyTool));
    parent.register(Arc::new(SuggestOptionsTool));
    let child = subagent::build_child_registry(&parent);
    assert!(
        !child.has_tool("session_notify") && !child.has_tool("suggest_options"),
        "child registry must strip interactive-only tools regardless of parent surface"
    );
}
