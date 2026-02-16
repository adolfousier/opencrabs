//! User-Defined Slash Commands
//!
//! Loads and saves user slash commands from a JSON file in the brain workspace.
//! Commands are merged with built-in slash commands for autocomplete.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A user-defined slash command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserCommand {
    /// Command name including the leading slash, e.g. "/deploy"
    pub name: String,

    /// Short description shown in autocomplete
    pub description: String,

    /// Action type: "prompt" sends to LLM, "system" displays inline
    #[serde(default = "default_action")]
    pub action: String,

    /// The prompt text or system message content
    pub prompt: String,
}

fn default_action() -> String {
    "prompt".to_string()
}

/// Loads and saves user-defined slash commands from a JSON file.
pub struct CommandLoader {
    path: PathBuf,
}

impl CommandLoader {
    /// Create a new CommandLoader with the given JSON file path.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Resolve the commands.json path from the brain path.
    pub fn from_brain_path(brain_path: &std::path::Path) -> Self {
        // commands.json lives at ~/.opencrabs/commands.json
        Self { path: brain_path.join("commands.json") }
    }

    /// Load user commands from JSON file. Returns empty vec if file doesn't exist.
    pub fn load(&self) -> Vec<UserCommand> {
        match std::fs::read_to_string(&self.path) {
            Ok(content) => match serde_json::from_str::<Vec<UserCommand>>(&content) {
                Ok(commands) => {
                    tracing::info!(
                        "Loaded {} user commands from {}",
                        commands.len(),
                        self.path.display()
                    );
                    commands
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse commands.json at {}: {}",
                        self.path.display(),
                        e
                    );
                    Vec::new()
                }
            },
            Err(_) => {
                tracing::debug!(
                    "No commands.json found at {} (this is normal)",
                    self.path.display()
                );
                Vec::new()
            }
        }
    }

    /// Save user commands to JSON file.
    pub fn save(&self, commands: &[UserCommand]) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(commands)?;
        std::fs::write(&self.path, json)?;
        tracing::info!(
            "Saved {} user commands to {}",
            commands.len(),
            self.path.display()
        );
        Ok(())
    }

    /// Generate a slash commands section for the system brain.
    pub fn commands_section(
        builtin: &[(&str, &str)],
        user_commands: &[UserCommand],
    ) -> String {
        let mut section = String::new();

        section.push_str("Built-in commands:\n");
        for (name, desc) in builtin {
            section.push_str(&format!("  {} — {}\n", name, desc));
        }

        if !user_commands.is_empty() {
            section.push_str("\nUser-defined commands:\n");
            for cmd in user_commands {
                section.push_str(&format!("  {} — {}\n", cmd.name, cmd.description));
            }
        }

        section
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_load_nonexistent() {
        let loader = CommandLoader::new(PathBuf::from("/nonexistent/commands.json"));
        let commands = loader.load();
        assert!(commands.is_empty());
    }

    #[test]
    fn test_save_and_load() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("commands.json");
        let loader = CommandLoader::new(path);

        let commands = vec![
            UserCommand {
                name: "/deploy".to_string(),
                description: "Deploy to staging".to_string(),
                action: "prompt".to_string(),
                prompt: "Run deploy.sh".to_string(),
            },
            UserCommand {
                name: "/test".to_string(),
                description: "Run tests".to_string(),
                action: "prompt".to_string(),
                prompt: "Run cargo test".to_string(),
            },
        ];

        loader.save(&commands).unwrap();
        let loaded = loader.load();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].name, "/deploy");
        assert_eq!(loaded[1].name, "/test");
    }

    #[test]
    fn test_commands_section() {
        let builtin = vec![("/help", "Show help"), ("/model", "Current model")];
        let user = vec![UserCommand {
            name: "/deploy".to_string(),
            description: "Deploy".to_string(),
            action: "prompt".to_string(),
            prompt: "deploy".to_string(),
        }];

        let section = CommandLoader::commands_section(&builtin, &user);
        assert!(section.contains("/help"));
        assert!(section.contains("/deploy"));
        assert!(section.contains("User-defined"));
    }
}
