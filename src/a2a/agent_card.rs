//! Agent Card generation for `.well-known/agent.json`.
//!
//! Builds an `AgentCard` from the running OpenCrabs configuration,
//! exposing available skills and capabilities to other A2A agents.

use crate::a2a::types::*;
use crate::brain::tools::registry::ToolRegistry;

/// Build the Agent Card for this OpenCrabs instance.
///
/// Skills are generated dynamically based on available tools in the registry.
pub fn build_agent_card(host: &str, port: u16, tool_registry: Option<&ToolRegistry>) -> AgentCard {
    let base_url = format!("http://{}:{}", host, port);

    let mut skills = vec![AgentSkill {
        id: "code-analysis".to_string(),
        name: "Code Analysis & Refactoring".to_string(),
        description: Some(
            "Analyze source code, identify issues, and suggest improvements.".to_string(),
        ),
        tags: vec![
            "code".to_string(),
            "analysis".to_string(),
            "refactoring".to_string(),
        ],
        examples: vec!["Analyze this Rust module for performance issues.".to_string()],
        input_modes: vec!["text/plain".to_string(), "application/json".to_string()],
        output_modes: vec!["text/plain".to_string(), "application/json".to_string()],
    }];

    // Research skill requires search tools
    let has_search = tool_registry
        .map(|r| r.has_tool("web_search") || r.has_tool("memory_search"))
        .unwrap_or(true);
    if has_search {
        skills.push(AgentSkill {
            id: "research".to_string(),
            name: "Deep Research".to_string(),
            description: Some(
                "Perform multi-source research using web search, memory search, and document analysis. Runs in read-only mode."
                    .to_string(),
            ),
            tags: vec![
                "research".to_string(),
                "analysis".to_string(),
                "synthesis".to_string(),
            ],
            examples: vec![
                "Research the latest developments in AI agent security.".to_string(),
            ],
            input_modes: vec!["text/plain".to_string()],
            output_modes: vec!["text/plain".to_string(), "application/json".to_string()],
        });
    }

    skills.push(AgentSkill {
        id: "debate".to_string(),
        name: "Multi-Agent Debate".to_string(),
        description: Some(
            "Participate in structured multi-round debates with other A2A agents using knowledge-enriched context."
                .to_string(),
        ),
        tags: vec![
            "debate".to_string(),
            "council".to_string(),
            "multi-agent".to_string(),
        ],
        examples: vec![
            "Debate the pros and cons of microservices vs monoliths.".to_string(),
        ],
        input_modes: vec!["text/plain".to_string(), "application/json".to_string()],
        output_modes: vec!["text/plain".to_string(), "application/json".to_string()],
    });

    AgentCard {
        name: format!("OpenCrabs Bee (v{})", crate::VERSION),
        description: Some(
            "High-performance AI orchestration agent with A2A protocol support. \
             Part of the Bee Colony multi-agent system."
                .to_string(),
        ),
        version: Some(crate::VERSION.to_string()),
        documentation_url: Some("https://github.com/adolfousier/opencrabs".to_string()),
        icon_url: None,
        supported_interfaces: vec![SupportedInterface {
            url: format!("{}/a2a/v1", base_url),
            protocol_binding: "JSONRPC".to_string(),
            protocol_version: Some("1.0".to_string()),
        }],
        provider: Some(AgentProvider {
            organization: "OpenCrabs Contributors".to_string(),
            url: Some("https://github.com/adolfousier/opencrabs".to_string()),
        }),
        capabilities: Some(AgentCapabilities {
            streaming: false,
            push_notifications: false,
            state_transition_history: true,
        }),
        skills,
        default_input_modes: vec!["text/plain".to_string(), "application/json".to_string()],
        default_output_modes: vec!["text/plain".to_string(), "application/json".to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_agent_card_default() {
        let card = build_agent_card("127.0.0.1", 18790, None);
        assert!(card.name.contains("OpenCrabs"));
        assert_eq!(card.skills.len(), 3);
        assert_eq!(
            card.supported_interfaces[0].url,
            "http://127.0.0.1:18790/a2a/v1"
        );
    }

    #[test]
    fn test_build_agent_card_with_registry() {
        let registry = ToolRegistry::new();
        let card = build_agent_card("127.0.0.1", 18790, Some(&registry));
        // No search tools registered, so no research skill
        assert_eq!(card.skills.len(), 2);
        assert_eq!(card.skills[0].id, "code-analysis");
        assert_eq!(card.skills[1].id, "debate");
    }
}
