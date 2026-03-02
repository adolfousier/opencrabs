//! Brain Loader & Prompt Builder
//!
//! Reads workspace markdown files and assembles the system brain dynamically
//! each turn, so edits to brain files take effect immediately.

use std::path::PathBuf;

/// Core brain files — always injected (personality + identity only).
const CORE_BRAIN_FILES: &[(&str, &str)] =
    &[("SOUL.md", "personality"), ("IDENTITY.md", "identity")];

/// Contextual brain files — loaded on demand via the `load_brain_file` tool.
pub(crate) const CONTEXTUAL_BRAIN_FILES: &[(&str, &str)] = &[
    ("USER.md", "user profile"),
    ("AGENTS.md", "workspace rules"),
    ("TOOLS.md", "tool notes"),
    ("SECURITY.md", "security policies"),
    ("MEMORY.md", "long-term memory"),
    ("BOOT.md", "startup config"),
    ("BOOTSTRAP.md", "bootstrap config"),
    ("HEARTBEAT.md", "heartbeat config"),
];

/// All brain files in assembly order — kept for `build_system_brain` (full mode).
const BRAIN_FILES: &[(&str, &str)] = &[
    ("SOUL.md", "personality"),
    ("IDENTITY.md", "identity"),
    ("USER.md", "user"),
    ("AGENTS.md", "agents"),
    ("TOOLS.md", "tools"),
    ("SECURITY.md", "security"),
    ("MEMORY.md", "memory"),
    ("BOOT.md", "boot"),
    ("BOOTSTRAP.md", "bootstrap"),
    ("HEARTBEAT.md", "heartbeat"),
];

/// Brain preamble — always present regardless of workspace contents.
const BRAIN_PREAMBLE: &str = r#"You are OpenCrabs, an AI orchestration agent with powerful tools to help with software development tasks.

IMPORTANT: You have access to tools for file operations and code exploration. USE THEM PROACTIVELY!

CRITICAL RULE: After calling tools and getting results, you MUST provide a final text response to the user.
DO NOT keep calling tools in a loop. Call the necessary tools, get results, then respond with text.

When asked to analyze or explore a codebase:
1. Use 'ls' tool with recursive=true to list all directories and files
2. Use 'glob' tool with patterns like "**/*.rs", "**/*.toml", "**/*.md" to find files
3. Use 'grep' tool to search for patterns, functions, or keywords in code
4. Use 'read_file' tool to read specific files you've identified
5. Use 'bash' tool for git operations like: git log, git diff, git branch

When asked to make changes:
1. Use 'read_file' first to understand the current code
2. Use 'edit_file' to modify existing files
3. Use 'write_file' to create new files
4. Use 'bash' to run tests or build commands

Available tools and their REQUIRED parameters (use exact parameter names):
- ls: List directory contents. Params: path (string), recursive (bool)
- glob: Find files matching patterns. Params: pattern (string, REQUIRED — e.g. "**/*.rs")
- grep: Search for text in files. Params: pattern (string, REQUIRED — the search text), path (string), regex (bool), case_insensitive (bool), file_pattern (string), limit (int), context (int)
- read_file: Read file contents. Params: path (string, REQUIRED)
- edit_file: Modify existing files. Params: path (string, REQUIRED), operation (string, REQUIRED)
- write_file: Create new files. Params: path (string, REQUIRED), content (string, REQUIRED)
- bash: Run shell commands. Params: command (string, REQUIRED)
- execute_code: Test code snippets. Params: language (string, REQUIRED), code (string, REQUIRED)
- web_search: Search the internet. Params: query (string, REQUIRED)
- http_request: Call external APIs. Params: method (string, REQUIRED), url (string, REQUIRED)
- task_manager: Track multi-step work. Params: operation (string, REQUIRED)
- session_context: Remember important facts. Params: operation (string, REQUIRED)
- session_search: Search across sessions. Params: operation (string, REQUIRED — "search" or "list"), query (string), n (int)
- plan: Create structured plans. Params: operation (string, REQUIRED)

CRITICAL: PLAN TOOL USAGE
When a user says "create a plan", "make a plan", or describes a complex multi-step task, you MUST use the plan tool immediately.
DO NOT write a text description of a plan. DO NOT explain what should be done. CALL THE TOOL.

Mandatory steps for plan creation:
1. IMMEDIATELY call plan tool with operation='create' to create a new plan
2. Call plan tool with operation='add_task' for each task (call multiple times)
   - IMPORTANT: The 'description' field MUST contain detailed implementation steps
   - Include: specific files to create/modify, functions to implement, commands to run
   - Format: Use numbered steps or bullet points for clarity
   - Be concrete: "Create Login.jsx component with email/password form fields and validation"
     NOT vague: "Create login component"
3. Call plan tool with operation='finalize' — this auto-approves the plan immediately
4. Begin executing tasks in order right away using start_task/complete_task — no waiting

NEVER generate text plans. ALWAYS use the plan tool for planning requests.

ALWAYS explore first before answering questions about a codebase. Don't guess - use the tools!"#;

/// Loads brain workspace files and assembles the system brain.
pub struct BrainLoader {
    workspace_path: PathBuf,
}

impl BrainLoader {
    /// Create a new BrainLoader with the given workspace path.
    pub fn new(workspace_path: PathBuf) -> Self {
        Self { workspace_path }
    }

    /// Resolve the brain path: `~/.opencrabs/`
    ///
    /// Brain files (SOUL.md, IDENTITY.md, etc.) live at the root of the
    /// OpenCrabs home directory for simplicity.
    pub fn resolve_path() -> PathBuf {
        crate::config::opencrabs_home()
    }

    /// Read a single markdown file from the workspace. Returns `None` if missing.
    pub fn load_file(&self, name: &str) -> Option<String> {
        let path = self.workspace_path.join(name);
        std::fs::read_to_string(&path).ok()
    }

    /// Build the full system brain from workspace files + brain preamble.
    ///
    /// Assembly order:
    /// 1. Brain preamble (hardcoded, always present)
    /// 2. SOUL.md — personality, tone, hard rules
    /// 3. IDENTITY.md — agent name, vibe, emoji
    /// 4. USER.md — who the human is
    /// 5. AGENTS.md — workspace rules, memory system, safety
    /// 6. TOOLS.md — environment-specific notes
    /// 7. MEMORY.md — long-term context
    /// 8. Runtime info — model, provider, working directory, OS, timestamp
    /// 9. Slash commands list (provided externally)
    pub fn build_system_brain(
        &self,
        runtime_info: Option<&RuntimeInfo>,
        slash_commands_section: Option<&str>,
    ) -> String {
        let mut prompt = String::with_capacity(8192);

        // 1. Brain preamble — always present
        prompt.push_str(BRAIN_PREAMBLE);
        prompt.push_str("\n\n");

        // 2-7. Brain workspace files (skip missing ones silently)
        for (filename, label) in BRAIN_FILES {
            if let Some(content) = self.load_file(filename) {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    prompt.push_str(&format!(
                        "--- {} ({}) ---\n{}\n\n",
                        filename, label, trimmed
                    ));
                }
            }
        }

        // 8. Runtime info
        if let Some(info) = runtime_info {
            prompt.push_str("--- Runtime Info ---\n");
            if let Some(ref model) = info.model {
                prompt.push_str(&format!("Model: {}\n", model));
            }
            if let Some(ref provider) = info.provider {
                prompt.push_str(&format!("Provider: {}\n", provider));
            }
            if let Some(ref wd) = info.working_directory {
                prompt.push_str(&format!("Working directory: {}\n", wd));
            }
            prompt.push_str(&format!("OS: {}\n", std::env::consts::OS));
            prompt.push_str(&format!(
                "Timestamp: {}\n",
                chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
            ));
            prompt.push('\n');
        }

        // 9. Slash commands list
        if let Some(commands_section) = slash_commands_section
            && !commands_section.is_empty()
        {
            prompt.push_str("--- Available Slash Commands ---\n");
            prompt.push_str(commands_section);
            prompt.push_str("\n\n");
        }

        prompt
    }

    /// Build a lean "core" system brain: only SOUL.md + IDENTITY.md are injected.
    ///
    /// All other brain files (USER.md, MEMORY.md, AGENTS.md, etc.) are listed in a
    /// "Available Context Files" index section so the agent knows they exist and can
    /// load them on demand via the `load_brain_file` tool — only when actually needed.
    ///
    /// This eliminates 10–20k token overhead from requests that don't need user profile,
    /// long-term memory, or policy files.
    pub fn build_core_brain(
        &self,
        runtime_info: Option<&RuntimeInfo>,
        slash_commands_section: Option<&str>,
    ) -> String {
        let mut prompt = String::with_capacity(4096);

        // 1. Brain preamble — always present
        prompt.push_str(BRAIN_PREAMBLE);
        prompt.push_str("\n\n");

        // 2. Core files only (SOUL.md + IDENTITY.md)
        for (filename, label) in CORE_BRAIN_FILES {
            if let Some(content) = self.load_file(filename) {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    prompt.push_str(&format!(
                        "--- {} ({}) ---\n{}\n\n",
                        filename, label, trimmed
                    ));
                }
            }
        }

        // 3. Memory index — list contextual files that exist on disk
        let available: Vec<(&str, &str)> = CONTEXTUAL_BRAIN_FILES
            .iter()
            .filter(|(name, _)| self.workspace_path.join(name).exists())
            .copied()
            .collect();

        if !available.is_empty() {
            prompt.push_str("--- Available Context Files ---\n");
            prompt.push_str(
                "The following brain files contain detailed context. \
                 Load them on demand using the `load_brain_file` tool when relevant — \
                 do NOT load them unless the request actually needs that context. \
                 To update or edit a brain file, use the `write_opencrabs_file` tool.\n\n",
            );
            for (name, desc) in &available {
                prompt.push_str(&format!("- **{}**: {}\n", name, desc));
            }
            // Guidance text: only mention files that actually exist on disk
            let has = |name: &str| available.iter().any(|(n, _)| *n == name);
            prompt.push_str("\nLoad proactively when:\n");
            if has("USER.md") {
                prompt.push_str("- User asks personal questions or preferences → load USER.md\n");
            }
            if has("MEMORY.md") {
                prompt.push_str(
                    "- Starting a project session or recalling past work → load MEMORY.md\n",
                );
            }
            if has("AGENTS.md") || has("SECURITY.md") {
                let files: Vec<&str> = ["AGENTS.md", "SECURITY.md"]
                    .iter()
                    .copied()
                    .filter(|n| has(n))
                    .collect();
                prompt.push_str(&format!(
                    "- Policy / rule / safety check needed → load {}\n",
                    files.join(" or ")
                ));
            }
            if has("TOOLS.md") {
                prompt
                    .push_str("- Working with environment-specific tool configs → load TOOLS.md\n");
            }
            prompt.push('\n');
        }

        // 4. Runtime info
        if let Some(info) = runtime_info {
            prompt.push_str("--- Runtime Info ---\n");
            if let Some(ref model) = info.model {
                prompt.push_str(&format!("Model: {}\n", model));
            }
            if let Some(ref provider) = info.provider {
                prompt.push_str(&format!("Provider: {}\n", provider));
            }
            if let Some(ref wd) = info.working_directory {
                prompt.push_str(&format!("Working directory: {}\n", wd));
            }
            prompt.push_str(&format!("OS: {}\n", std::env::consts::OS));
            prompt.push_str(&format!(
                "Timestamp: {}\n",
                chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
            ));
            prompt.push('\n');
        }

        // 5. Slash commands list
        if let Some(commands_section) = slash_commands_section
            && !commands_section.is_empty()
        {
            prompt.push_str("--- Available Slash Commands ---\n");
            prompt.push_str(commands_section);
            prompt.push_str("\n\n");
        }

        prompt
    }
}

/// Runtime information injected into the system brain.
#[derive(Debug, Clone, Default)]
pub struct RuntimeInfo {
    pub model: Option<String>,
    pub provider: Option<String>,
    pub working_directory: Option<String>,
}

#[cfg(test)]
#[path = "prompt_builder_tests.rs"]
mod prompt_builder_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_build_prompt_no_files() {
        let dir = TempDir::new().unwrap();
        let loader = BrainLoader::new(dir.path().to_path_buf());
        let prompt = loader.build_system_brain(None, None);

        // Should contain brain preamble even with no brain files
        assert!(prompt.contains("You are OpenCrabs"));
        assert!(prompt.contains("CRITICAL RULE"));
    }

    #[test]
    fn test_build_prompt_with_soul() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("SOUL.md"), "I am a helpful crab.").unwrap();

        let loader = BrainLoader::new(dir.path().to_path_buf());
        let prompt = loader.build_system_brain(None, None);

        assert!(prompt.contains("You are OpenCrabs"));
        assert!(prompt.contains("I am a helpful crab."));
        assert!(prompt.contains("SOUL.md"));
    }

    #[test]
    fn test_build_prompt_with_runtime_info() {
        let dir = TempDir::new().unwrap();
        let loader = BrainLoader::new(dir.path().to_path_buf());
        let info = RuntimeInfo {
            model: Some("claude-sonnet-4-20250514".to_string()),
            provider: Some("anthropic".to_string()),
            working_directory: Some("/home/user/project".to_string()),
        };
        let prompt = loader.build_system_brain(Some(&info), None);

        assert!(prompt.contains("claude-sonnet-4-20250514"));
        assert!(prompt.contains("anthropic"));
        assert!(prompt.contains("/home/user/project"));
    }

    #[test]
    fn test_skips_empty_files() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("SOUL.md"), "  \n  ").unwrap();

        let loader = BrainLoader::new(dir.path().to_path_buf());
        let prompt = loader.build_system_brain(None, None);

        // Should NOT contain SOUL.md section header for empty content
        assert!(!prompt.contains("SOUL.md"));
    }
}
