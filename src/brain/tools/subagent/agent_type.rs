//! Agent type definitions for typed sub-agent spawning.
//!
//! Each agent type defines a role with a specific system prompt and tool filter,
//! enabling specialized sub-agents (explore, plan, code, research) instead of
//! generic "do everything" agents.

use crate::brain::tools::ToolRegistry;

/// Tools that sub-agents must NEVER have access to (prevents recursion / dangerous ops).
const ALWAYS_EXCLUDED: &[&str] = &[
    "spawn_agent",
    "resume_agent",
    "wait_agent",
    "send_input",
    "close_agent",
    "rebuild",
    "evolve",
];

/// Built-in agent type identifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentType {
    /// General-purpose agent — inherits parent's full tool set (minus recursive/dangerous).
    General,
    /// Fast codebase exploration — read-only tools, no writes.
    Explore,
    /// Architecture planning — read + analysis tools, no mutations.
    Plan,
    /// Code implementation — full write access, focused on making changes.
    Code,
    /// Research — web search + read, no file modifications.
    Research,
}

impl AgentType {
    /// Parse an agent type from a string. Returns General for unknown types.
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "explore" | "search" | "find" => Self::Explore,
            "plan" | "architect" | "design" => Self::Plan,
            "code" | "implement" | "write" => Self::Code,
            "research" | "web" | "lookup" => Self::Research,
            _ => Self::General,
        }
    }

    /// Human-readable name for this agent type.
    pub fn label(&self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Explore => "explore",
            Self::Plan => "plan",
            Self::Code => "code",
            Self::Research => "research",
        }
    }

    /// System prompt prefix injected before the user's task prompt.
    pub fn system_prompt(&self) -> &'static str {
        match self {
            Self::General => {
                "You are a general-purpose sub-agent. Complete the task using all available tools."
            }
            Self::Explore => {
                "You are a codebase exploration agent. Your job is to find files, search code, \
                 and answer questions about the codebase structure. Do NOT modify any files. \
                 Use glob, grep, read, and ls to navigate efficiently. Be thorough but fast."
            }
            Self::Plan => {
                "You are an architecture planning agent. Analyze the codebase and produce a \
                 structured implementation plan. Read files to understand the current state, \
                 then output a clear step-by-step plan with file paths and specific changes. \
                 Do NOT make any code changes yourself."
            }
            Self::Code => {
                "You are a code implementation agent. Make the requested changes using write, \
                 edit, and bash tools. Be precise, follow existing code patterns, and run \
                 clippy/tests after changes."
            }
            Self::Research => {
                "You are a research agent. Search the web and read documentation to answer \
                 questions. Summarize findings concisely. Do NOT modify any local files."
            }
        }
    }

    /// Tools this agent type is allowed to use (empty = all from parent minus ALWAYS_EXCLUDED).
    fn allowed_tools(&self) -> Option<&'static [&'static str]> {
        match self {
            Self::General | Self::Code => None, // all parent tools minus exclusions
            Self::Explore => Some(&["read_file", "glob", "grep", "ls"]),
            Self::Plan => Some(&["read_file", "glob", "grep", "ls", "bash"]),
            Self::Research => Some(&[
                "read_file",
                "glob",
                "grep",
                "ls",
                "web_search",
                "exa_search",
                "brave_search",
                "http_client",
            ]),
        }
    }

    /// Build a filtered tool registry by copying tools from the parent registry.
    ///
    /// - General/Code: gets everything the parent has minus recursive/dangerous tools
    /// - Explore/Plan/Research: gets only the tools in their allowed list
    pub fn build_registry(&self, parent: &ToolRegistry) -> ToolRegistry {
        let child = ToolRegistry::new();

        let allowed = self.allowed_tools();

        for name in parent.list_tools() {
            // Always exclude recursive/dangerous tools
            if ALWAYS_EXCLUDED.contains(&name.as_str()) {
                continue;
            }

            // If this type has an allow-list, check it
            if let Some(allow) = allowed
                && !allow.contains(&name.as_str())
            {
                continue;
            }

            if let Some(tool) = parent.get(&name) {
                child.register(tool);
            }
        }

        child
    }
}
