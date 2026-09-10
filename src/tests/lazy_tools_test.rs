//! Lazy tool-schema loading: a request ships only the CORE tool schemas plus
//! `tool_search` (and whatever EXTENDED tools the session has activated), so a
//! tool-light turn doesn't pay the ~20k tokens of all ~95 schemas. These pin
//! the catalog tiering and the registry's filter/search/activate primitives.

use crate::brain::tools::catalog;
use crate::brain::tools::error::Result;
use crate::brain::tools::registry::ToolRegistry;
use crate::brain::tools::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashSet;
use uuid::Uuid;

/// Minimal tool with a fixed name/description for registry tests.
struct MockTool {
    name: &'static str,
    desc: &'static str,
}

#[async_trait]
impl Tool for MockTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        self.desc
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![]
    }
    async fn execute(&self, _input: Value, _ctx: &ToolExecutionContext) -> Result<ToolResult> {
        Ok(ToolResult::success("ok".to_string()))
    }
}

fn registry_with(tools: &[(&'static str, &'static str)]) -> ToolRegistry {
    let reg = ToolRegistry::new();
    for (name, desc) in tools {
        reg.register(std::sync::Arc::new(MockTool { name, desc }));
    }
    reg
}

#[test]
fn catalog_splits_core_from_extended() {
    assert!(catalog::is_core("bash"));
    assert!(catalog::is_core("read_file"));
    assert!(catalog::is_core("tool_search"));
    assert!(catalog::is_core("analyze_image"));
    assert!(catalog::is_core("analyze_video"));
    assert!(!catalog::is_core("browser_navigate"));
    assert!(!catalog::is_core("telegram_send"));

    assert_eq!(catalog::tool_category("bash"), "core");
    assert_eq!(catalog::tool_category("analyze_image"), "core");
    assert_eq!(catalog::tool_category("analyze_video"), "core");
    assert_eq!(catalog::tool_category("browser_click"), "browser");
    assert_eq!(catalog::tool_category("telegram_send"), "channels");
    assert_eq!(catalog::tool_category("spawn_agent"), "agents");
    assert_eq!(catalog::tool_category("generate_image"), "media");
    assert_eq!(catalog::tool_category("self_improve"), "system");
}

#[test]
fn tool_inventory_names_are_extended_not_core() {
    // The flat inventory (#448) advertises tools whose schemas are withheld
    // until tool_search activates them. Listing a CORE tool there would be
    // wrong (core schemas already ship), and a duplicate name is copy-paste
    // drift. Both are cheap mistakes this guards against.
    let mut seen = std::collections::HashSet::new();
    for (category, names) in catalog::EXTENDED_TOOL_INVENTORY {
        assert!(!names.is_empty(), "inventory group '{category}' is empty");
        for name in *names {
            assert!(
                !catalog::is_core(name),
                "inventory lists core tool '{name}' under '{category}' — core schemas already ship, it must not be advertised for tool_search"
            );
            assert!(
                seen.insert(*name),
                "'{name}' appears twice in the tool inventory"
            );
        }
    }
}

#[test]
fn tool_access_prompt_pairs_directive_with_roster() {
    // #449: the behavioural nudge only works if the tool_search directive AND a
    // concrete roster reach the model together. Pin that the assembled section
    // carries both, so a formatter/wiring regression can't silently drop either.
    let prompt = catalog::tool_access_prompt(false);
    assert!(
        prompt.contains("tool_search"),
        "tool-access prompt must keep the tool_search directive"
    );
    assert!(
        prompt.contains("AVAILABLE EXTENDED TOOLS"),
        "tool-access prompt must include the flat inventory header"
    );
    // A representative name from several groups must render, proving the
    // inventory formatter walked the whole const, not just the first group.
    for name in [
        "telegram_send",
        "browser_navigate",
        "spawn_agent",
        "generate_document",
        "cron_manage",
        "self_improve",
    ] {
        assert!(
            prompt.contains(name),
            "tool-access prompt is missing '{name}' from the inventory"
        );
    }
}

#[test]
fn filtered_definitions_include_core_and_active_only() {
    let reg = registry_with(&[
        ("bash", "run a shell command"),
        ("read_file", "read a file"),
        ("browser_navigate", "open a web page"),
        ("telegram_send", "send a telegram message"),
    ]);

    // No activations → only the core tools (bash, read_file) ship.
    let core_only = reg.get_tool_definitions_filtered(&HashSet::new());
    let names: HashSet<&str> = core_only.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains("bash") && names.contains("read_file"));
    assert!(
        !names.contains("browser_navigate") && !names.contains("telegram_send"),
        "extended tools must be withheld until activated; got {names:?}"
    );

    // Activate one extended tool → it now ships alongside core.
    let active: HashSet<String> = ["browser_navigate".to_string()].into_iter().collect();
    let with_browser = reg.get_tool_definitions_filtered(&active);
    let names: HashSet<&str> = with_browser.iter().map(|t| t.name.as_str()).collect();
    assert!(
        names.contains("browser_navigate"),
        "activated tool must ship"
    );
    assert!(
        !names.contains("telegram_send"),
        "un-activated extended tool must still be withheld"
    );
}

#[test]
fn search_ranks_name_and_category_hits_above_description() {
    let reg = registry_with(&[
        ("bash", "run a shell command"),
        ("browser_navigate", "open a web page in a browser"),
        ("telegram_send", "send a message to a telegram chat"),
        ("generate_image", "create an image from a text prompt"),
    ]);

    // Query by intent — the telegram tool should surface, core tools never do.
    let hits = reg.search_tools("send a telegram message", 8);
    let names: Vec<&str> = hits.iter().map(|(n, ..)| n.as_str()).collect();
    assert!(names.contains(&"telegram_send"), "got {names:?}");
    assert!(
        !names.contains(&"bash"),
        "core tools are never returned by tool_search"
    );

    // Category query works too.
    let browser = reg.search_tools("browser", 8);
    assert!(browser.iter().any(|(n, ..)| n == "browser_navigate"));
}

#[test]
fn activate_tools_is_per_session() {
    let reg = registry_with(&[("browser_navigate", "open a web page")]);
    let s1 = Uuid::new_v4();
    let s2 = Uuid::new_v4();

    reg.activate_tools(s1, ["browser_navigate".to_string()]);
    assert!(reg.active_tools(s1).contains("browser_navigate"));
    assert!(
        reg.active_tools(s2).is_empty(),
        "activation must not leak across sessions"
    );
}

#[test]
fn active_set_is_lru_bounded() {
    use crate::brain::tools::registry::MAX_ACTIVE_EXTENDED;
    let reg = crate::brain::tools::ToolRegistry::new();
    let s = Uuid::new_v4();

    // Activate more than the cap, oldest first (t0 is least-recently-touched).
    let n = MAX_ACTIVE_EXTENDED + 6;
    for i in 0..n {
        reg.activate_tools(s, [format!("t{i}")]);
    }
    let active = reg.active_tools(s);
    assert_eq!(
        active.len(),
        MAX_ACTIVE_EXTENDED,
        "active set must be capped at MAX_ACTIVE_EXTENDED"
    );
    // The 6 oldest were evicted; the newest MAX_ACTIVE_EXTENDED survive.
    assert!(
        !active.contains("t0"),
        "least-recently-used must be evicted"
    );
    assert!(
        active.contains(&format!("t{}", n - 1)),
        "most-recently-activated must survive"
    );

    // Touching an about-to-be-evicted tool refreshes its recency, so a further
    // activation evicts a different (now-older) one instead.
    let victim_before = format!("t{}", n - MAX_ACTIVE_EXTENDED); // oldest survivor
    reg.activate_tools(s, [victim_before.clone()]); // refresh it
    reg.activate_tools(s, ["fresh".to_string()]); // push over cap again
    let active = reg.active_tools(s);
    assert_eq!(active.len(), MAX_ACTIVE_EXTENDED);
    assert!(
        active.contains(&victim_before),
        "a refreshed tool must not be the one evicted"
    );
    assert!(active.contains("fresh"));
}

#[test]
fn vision_tools_are_core_when_registered() {
    // When vision is configured, analyze_image/analyze_video are registered
    // and should appear in the core set without needing tool_search activation.
    let reg = registry_with(&[
        ("bash", "run a shell command"),
        ("analyze_image", "analyze an image with vision"),
        ("analyze_video", "analyze a video with vision"),
        ("browser_navigate", "open a web page"),
    ]);

    let core_only = reg.get_tool_definitions_filtered(&HashSet::new());
    let names: HashSet<&str> = core_only.iter().map(|t| t.name.as_str()).collect();

    assert!(
        names.contains("analyze_image"),
        "analyze_image should be in core set when registered; got {names:?}"
    );
    assert!(
        names.contains("analyze_video"),
        "analyze_video should be in core set when registered; got {names:?}"
    );
    assert!(
        !names.contains("browser_navigate"),
        "browser_navigate is extended, should not be in core set"
    );
}
