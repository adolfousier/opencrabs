use crate::brain::prompt_builder::{BrainLoader, RuntimeInfo};
use crate::brain::tools::error::collapse_home;
use std::path::Path;

pub const READ_ONLY_CAPABILITY_NOTE: &str = "[Capability note: This sub-agent runs with a read-restricted tool registry (#1173) — file reads, search, and web research only. Tools for shell execution, file writes, and further sub-agents are absent.]\n\n";

pub const LEAN_BRAIN_CONTEXT_NOTE: &str = "[Context note: This sub-agent runs without pre-injected workspace brain files (SOUL.md, USER.md, AGENTS.md). Call load_brain_file(name, query) to consult workspace conventions when relevant, or note missing guidelines if needed.]\n\n";

/// Builds the system brain string for a child agent if include_brain is true.
pub fn child_system_brain(
    include_brain: bool,
    child_dir: &Path,
    model: Option<&str>,
    provider: Option<&str>,
) -> Option<String> {
    if !include_brain {
        return None;
    }
    let runtime_info = RuntimeInfo {
        model: model.map(|s| s.to_string()),
        provider: provider.map(|s| s.to_string()),
        working_directory: Some(collapse_home(child_dir)),
    };
    let brain_path = BrainLoader::resolve_path();
    let loader = BrainLoader::new(brain_path);
    Some(loader.build_core_brain(Some(&runtime_info)))
}

/// Constructs the full prompt delivered to a child agent, stacking capability and context notes.
pub fn child_prompt(read_only: bool, include_brain: bool, prompt: &str) -> String {
    let mut prefix = String::new();
    if read_only {
        prefix.push_str(READ_ONLY_CAPABILITY_NOTE);
    }
    if !include_brain {
        prefix.push_str(LEAN_BRAIN_CONTEXT_NOTE);
    }
    format!("{}{}", prefix, prompt)
}

/// Human-readable label for spawn response reporting.
pub fn brain_status_label(include_brain: bool) -> &'static str {
    if include_brain {
        "core attached (SOUL/USER/AGENTS)"
    } else {
        "lean (none pre-injected; use load_brain_file)"
    }
}
