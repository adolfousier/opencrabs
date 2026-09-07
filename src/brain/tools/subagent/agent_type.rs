//! Sub-agent tool-registry construction.
//!
//! Typed agent roles (`AgentType`) were removed (#1173): a child is now
//! either read-restricted or not, decided solely by the parent's explicit
//! `read_only` grant at spawn time. The old per-type allow-lists and canned
//! system-prompt preambles are gone — restricted children get their tool set
//! from [`restrict_registry_to_read_only`](crate::brain::tools::plan_gate::restrict_registry_to_read_only)
//! (#649) and one factual capability line instead of role-play text.
//!
//! Deprecated `agent_type` values from the typed API still resolve through
//! [`map_deprecated_agent_type`], which maps each known value to its
//! historical *effective* grant (Explore/Research were read-only; Plan carried
//! bash, so it was write-capable). Unknown values fail closed rather than
//! silently escalating to full write access.

use crate::brain::tools::catalog;
use crate::brain::tools::registry::ToolRegistry;
use crate::brain::tools::tool_search::ToolSearchTool;
use std::sync::Arc;

/// Tools that sub-agents must NEVER have access to (prevents recursion / dangerous ops).
///
/// #129 (owner ruling C, 2026-09-07): `session_notify` and `suggest_options`
/// are in this list because a sub-agent IS a headless session — its only
/// user-visible output is the final message relayed by the harness
/// (proven live-fire 2026-09-07; a parked session_notify verdict dies with
/// the one-shot process). Children inherit the parent registry verbatim, so
/// an interactive parent's copies of these tools would leak into the child
/// without this strip.
pub(crate) const ALWAYS_EXCLUDED: &[&str] = &[
    "spawn_agent",
    "resume_agent",
    "wait_agent",
    "send_input",
    "close_agent",
    "team_create",
    "team_delete",
    "team_broadcast",
    "rebuild",
    "evolve",
    "session_notify",
    "suggest_options",
];

/// Build a child registry from the parent's, minus recursive/dangerous tools.
///
/// This is the FULL-access child baseline. When the parent granted only read
/// access, the caller additionally applies plan_gate's
/// `restrict_registry_to_read_only` on the returned registry.
///
/// Returns an [`Arc`] because callers hand the registry to `AgentService`
/// wrapped anyway — and because [`ToolSearchTool`] must be RE-BOUND to this
/// child Arc (#1210): the parent's search-tool instance captured the parent's
/// registry at construction. Copying that instance here made a child's
/// `tool_search` activate schemas in the PARENT's registry under the child's
/// session id, while the child's request builder read active tools from its
/// OWN registry — so a discovered tool's schema never rode on the child's
/// next request, and well-behaved models looped re-searching forever instead
/// of emitting calls for undeclared tools (observed live on two models across
/// two providers; see upstream #1210). Re-binding per level also keeps the
/// fix correct for nested spawns: each generation's search tool points at
/// its own registry.
pub fn build_child_registry(parent: &ToolRegistry) -> Arc<ToolRegistry> {
    let child = Arc::new(ToolRegistry::new());

    for name in parent.list_tools() {
        if ALWAYS_EXCLUDED.contains(&name.as_str()) {
            continue;
        }

        if name == catalog::TOOL_SEARCH_NAME {
            // Fresh instance bound to THIS registry, so the child's searches
            // land where the child's requests read (#1210).
            // Bound WEAKLY: the registry owns this tool, so handing it a
            // strong Arc back closes a registry -> tool -> registry cycle and
            // the child registry is never freed. One leaked registry per
            // spawn is a steep price for the rebinding.
            child.register(Arc::new(ToolSearchTool::new(&child)));
            continue;
        }

        if let Some(tool) = parent.get(&name) {
            child.register(tool);
        }
    }

    child
}

/// Resolve a deprecated `agent_type` string to its historical effective grant.
///
/// Returns `Ok(read_only)` where `read_only` reflects what the old typed role
/// actually allowed, or `Err(explanation)` for unrecognized values — failing
/// closed so a typo'd type can never silently become a full-write child.
///
/// Every call site MUST surface the deprecation loudly (warn log + schema-era
/// note in the spawn result); this function intentionally does not log, so
/// tests can assert on call-site behavior.
pub fn map_deprecated_agent_type(raw: &str) -> Result<bool, String> {
    match raw.trim().to_lowercase().as_str() {
        // Historically read-only roles (explore aliases included).
        "explore" | "search" | "find" | "research" | "web" | "lookup" => Ok(true),
        // Historically write-capable roles — note `plan` carried bash, so it
        // was NOT read-only despite its analysis-only description.
        "general" | "plan" | "architect" | "design" | "code" | "implement" | "write" => Ok(false),
        other => Err(format!(
            "Unknown agent_type '{other}'. Typed agent roles were removed \
             (#1173); pass read_only=true/false explicitly instead."
        )),
    }
}
