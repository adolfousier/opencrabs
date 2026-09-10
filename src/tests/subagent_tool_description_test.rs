//! Sentinel: every sub-agent spawn/resume tool description must
//! mention `subagent_provider` / `subagent_model` so the LLM (and
//! anyone reading the tool schema) knows how to control the child
//! agent's provider and model.
//!
//! Context (issue #152, 2026-06-02): a user reported that
//! `spawn_agent` had "no way to specify which model a sub-agent
//! should use — it always inherits the parent session's model." The
//! claim was wrong on its face — `config.agent.subagent_provider`
//! and `config.agent.subagent_model` are read by spawn.rs:120-130,
//! resume.rs:130-134, and team/create.rs:122-162 to override
//! the parent's provider for every spawned child. But none of the
//! tool descriptions said so, so the LLM (which only sees the tool
//! schema, never the README) genuinely had no way to answer "how do
//! I run sub-agents on a different model?" from its prompt context
//! alone.
//!
//! These tests pin the config-mention in each tool description so a
//! future trim of the description text can't silently re-open the
//! gap and re-trigger the same misunderstanding.

use crate::brain::tools::Tool;
use crate::brain::tools::subagent::SpawnAgentTool;

#[test]
fn spawn_agent_description_mentions_subagent_provider_config() {
    let tool = SpawnAgentTool::new(
        std::sync::Arc::new(crate::brain::tools::subagent::SubAgentManager::new()),
        std::sync::Arc::new(crate::brain::tools::ToolRegistry::new()),
    );
    let desc = tool.description();
    assert!(
        desc.contains("subagent_provider"),
        "spawn_agent description must name `subagent_provider` so the LLM knows the \
         config key to point users at. Without this, the model concludes there's no \
         way to control child-agent provider selection (issue #152). \
         \nCurrent description: {desc}"
    );
    assert!(
        desc.contains("subagent_model"),
        "spawn_agent description must also name `subagent_model` so the model key \
         is discoverable from the same schema."
    );
}

#[test]
fn resume_agent_description_mentions_subagent_provider_config() {
    use crate::brain::tools::subagent::ResumeAgentTool;
    let tool = ResumeAgentTool::new(
        std::sync::Arc::new(crate::brain::tools::subagent::SubAgentManager::new()),
        std::sync::Arc::new(crate::brain::tools::ToolRegistry::new()),
    );
    let desc = tool.description();
    assert!(
        desc.contains("subagent_provider") && desc.contains("subagent_model"),
        "resume_agent description must name both config keys so the LLM knows \
         resume follows the same routing rule as spawn. \nCurrent description: {desc}"
    );
}

#[test]
fn team_create_description_mentions_subagent_provider_config() {
    use crate::brain::tools::subagent::{TeamCreateTool, TeamManager};
    let tool = TeamCreateTool::new(
        std::sync::Arc::new(crate::brain::tools::subagent::SubAgentManager::new()),
        std::sync::Arc::new(TeamManager::new()),
        std::sync::Arc::new(crate::brain::tools::ToolRegistry::new()),
    );
    let desc = tool.description();
    // Accept either the explicit `subagent_provider`/`subagent_model`
    // form OR the wildcard `subagent_*` shorthand — both unambiguously
    // point a reader at the same config keys, and the wildcard form
    // is appropriate now that the description also covers per-member
    // overrides (so spelling out both keys twice would be noise).
    let names_both = desc.contains("subagent_provider") && desc.contains("subagent_model");
    let names_wildcard = desc.contains("subagent_*");
    assert!(
        names_both || names_wildcard,
        "team_create description must reference the config keys (either \
         `subagent_provider`+`subagent_model` or the `subagent_*` shorthand) \
         so the LLM knows where to point users for global routing. \
         \nCurrent description: {desc}"
    );
}

#[test]
fn spawn_agent_schema_exposes_per_call_provider_and_model() {
    // Issue #152 follow-up: per-call override. The schema must
    // expose `provider` and `model` as optional input params so an
    // LLM driving a skill can route each spawn to a different model
    // without editing config between calls.
    let tool = SpawnAgentTool::new(
        std::sync::Arc::new(crate::brain::tools::subagent::SubAgentManager::new()),
        std::sync::Arc::new(crate::brain::tools::ToolRegistry::new()),
    );
    let schema = tool.input_schema();
    let props = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("schema must have properties");
    assert!(
        props.contains_key("provider"),
        "spawn_agent must expose an optional `provider` param for per-call \
         routing (issue #152). Without it, skills can't orchestrate steps \
         that each use a different provider."
    );
    assert!(
        props.contains_key("model"),
        "spawn_agent must expose an optional `model` param for per-call \
         routing (issue #152). Without it, skills can't say \"this step uses \
         GLM, that step uses Deepseek\" inside one run."
    );
    // Neither is required — they're overrides on top of config defaults.
    let required = schema
        .get("required")
        .and_then(|v| v.as_array())
        .expect("schema must have required");
    let required_keys: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        !required_keys.contains(&"provider"),
        "`provider` must be optional — config and parent inheritance \
         remain valid fallbacks"
    );
    assert!(
        !required_keys.contains(&"model"),
        "`model` must be optional"
    );
}

#[test]
fn resume_agent_schema_exposes_per_call_provider_and_model() {
    use crate::brain::tools::subagent::ResumeAgentTool;
    let tool = ResumeAgentTool::new(
        std::sync::Arc::new(crate::brain::tools::subagent::SubAgentManager::new()),
        std::sync::Arc::new(crate::brain::tools::ToolRegistry::new()),
    );
    let schema = tool.input_schema();
    let props = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("schema must have properties");
    assert!(
        props.contains_key("provider") && props.contains_key("model"),
        "resume_agent must expose `provider` and `model` overrides — \
         a resume might want to escalate to a stronger model than the \
         original spawn (issue #152)"
    );
}

#[test]
fn team_create_each_member_can_pick_its_own_provider_and_model() {
    use crate::brain::tools::subagent::{TeamCreateTool, TeamManager};
    let tool = TeamCreateTool::new(
        std::sync::Arc::new(crate::brain::tools::subagent::SubAgentManager::new()),
        std::sync::Arc::new(TeamManager::new()),
        std::sync::Arc::new(crate::brain::tools::ToolRegistry::new()),
    );
    let schema = tool.input_schema();
    let agent_props = schema
        .pointer("/properties/agents/items/properties")
        .and_then(|v| v.as_object())
        .expect("agents items must have properties");
    assert!(
        agent_props.contains_key("provider") && agent_props.contains_key("model"),
        "team_create's per-agent item schema must expose `provider` and \
         `model` so a single team_create call can spawn members on \
         different models — the canonical use case from issue #152 \
         (plan with GLM, code with Deepseek, review with Kimi)"
    );
}

#[test]
fn spawn_agent_schema_exposes_include_brain_param() {
    let tool = SpawnAgentTool::new(
        std::sync::Arc::new(crate::brain::tools::subagent::SubAgentManager::new()),
        std::sync::Arc::new(crate::brain::tools::ToolRegistry::new()),
    );
    let schema = tool.input_schema();
    let props = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("schema must have properties");
    assert!(
        props.contains_key("include_brain"),
        "spawn_agent must expose optional `include_brain` parameter (#145)"
    );
}

#[test]
fn readme_documents_subagent_provider_and_model_keys() {
    // Bundled at compile time so the test is hermetic and a doc tidy-up
    // that drops the section can't slip past CI.
    const README: &str = include_str!("../../README.md");
    assert!(
        README.contains("subagent_provider"),
        "README must document `subagent_provider` so users searching the docs \
         find the config key. Issue #152 came from a user who didn't know it existed."
    );
    assert!(
        README.contains("subagent_model"),
        "README must document `subagent_model` alongside subagent_provider."
    );
}
