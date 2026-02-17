//! User-Defined Slash Commands
//!
//! Loads and saves user slash commands from TOML (with JSON fallback/migration).
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

/// TOML wrapper: `[[commands]]` array
#[derive(Debug, Serialize, Deserialize)]
struct CommandsFile {
    #[serde(default)]
    commands: Vec<UserCommand>,
}

/// Loads and saves user-defined slash commands (TOML primary, JSON fallback).
pub struct CommandLoader {
    /// Path to commands.toml
    toml_path: PathBuf,
    /// Path to legacy commands.json (for migration)
    json_path: PathBuf,
}

impl CommandLoader {
    /// Create a new CommandLoader pointing at a specific TOML file path.
    pub fn new(path: PathBuf) -> Self {
        let json_path = path.with_extension("json");
        Self { toml_path: path, json_path }
    }

    /// Resolve the commands paths from the brain path.
    pub fn from_brain_path(brain_path: &std::path::Path) -> Self {
        Self {
            toml_path: brain_path.join("commands.toml"),
            json_path: brain_path.join("commands.json"),
        }
    }

    /// Load user commands. Priority: TOML → JSON (with auto-migration) → empty.
    pub fn load(&self) -> Vec<UserCommand> {
        // 1. Try TOML first
        if let Ok(content) = std::fs::read_to_string(&self.toml_path) {
            match toml::from_str::<CommandsFile>(&content) {
                Ok(file) => {
                    tracing::info!(
                        "Loaded {} user commands from {}",
                        file.commands.len(),
                        self.toml_path.display()
                    );
                    return file.commands;
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse commands.toml at {}: {}",
                        self.toml_path.display(),
                        e
                    );
                }
            }
        }

        // 2. Fall back to JSON and auto-migrate
        if let Ok(content) = std::fs::read_to_string(&self.json_path) {
            match serde_json::from_str::<Vec<UserCommand>>(&content) {
                Ok(commands) => {
                    tracing::info!(
                        "Loaded {} user commands from legacy {} — migrating to TOML",
                        commands.len(),
                        self.json_path.display()
                    );
                    // Auto-migrate: save as TOML
                    if let Err(e) = self.save(&commands) {
                        tracing::warn!("Failed to auto-migrate commands to TOML: {}", e);
                    } else {
                        tracing::info!(
                            "Migrated commands.json → {}",
                            self.toml_path.display()
                        );
                    }
                    return commands;
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse commands.json at {}: {}",
                        self.json_path.display(),
                        e
                    );
                }
            }
        }

        tracing::debug!(
            "No commands file found at {} (this is normal)",
            self.toml_path.display()
        );
        Vec::new()
    }

    /// Save user commands to TOML file.
    pub fn save(&self, commands: &[UserCommand]) -> Result<()> {
        if let Some(parent) = self.toml_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = CommandsFile { commands: commands.to_vec() };
        let toml_str = toml::to_string_pretty(&file)?;
        std::fs::write(&self.toml_path, toml_str)?;
        tracing::info!(
            "Saved {} user commands to {}",
            commands.len(),
            self.toml_path.display()
        );
        Ok(())
    }

    /// Add a single command, preserving existing ones.
    pub fn add_command(&self, command: UserCommand) -> Result<()> {
        let mut commands = self.load();
        // Replace if same name exists
        if let Some(pos) = commands.iter().position(|c| c.name == command.name) {
            commands[pos] = command;
        } else {
            commands.push(command);
        }
        self.save(&commands)
    }

    /// Remove a command by name. Returns true if found and removed.
    pub fn remove_command(&self, name: &str) -> Result<bool> {
        let mut commands = self.load();
        let len_before = commands.len();
        commands.retain(|c| c.name != name);
        let removed = commands.len() < len_before;
        if removed {
            self.save(&commands)?;
        }
        Ok(removed)
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
        let loader = CommandLoader::new(PathBuf::from("/nonexistent/commands.toml"));
        let commands = loader.load();
        assert!(commands.is_empty());
    }

    #[test]
    fn test_save_and_load_toml() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("commands.toml");
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
    fn test_json_migration() {
        let dir = TempDir::new().unwrap();
        let json_path = dir.path().join("commands.json");
        let toml_path = dir.path().join("commands.toml");

        // Write legacy JSON
        let commands = vec![UserCommand {
            name: "/legacy".to_string(),
            description: "Legacy command".to_string(),
            action: "prompt".to_string(),
            prompt: "do legacy stuff".to_string(),
        }];
        let json = serde_json::to_string_pretty(&commands).unwrap();
        std::fs::write(&json_path, json).unwrap();

        // Load should find JSON and auto-migrate
        let loader = CommandLoader::new(toml_path.clone());
        let loaded = loader.load();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "/legacy");

        // TOML file should now exist
        assert!(toml_path.exists());

        // Loading again should use TOML
        let loaded2 = loader.load();
        assert_eq!(loaded2.len(), 1);
    }

    #[test]
    fn test_add_command() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("commands.toml");
        let loader = CommandLoader::new(path);

        loader.add_command(UserCommand {
            name: "/first".to_string(),
            description: "First".to_string(),
            action: "prompt".to_string(),
            prompt: "first".to_string(),
        }).unwrap();

        loader.add_command(UserCommand {
            name: "/second".to_string(),
            description: "Second".to_string(),
            action: "prompt".to_string(),
            prompt: "second".to_string(),
        }).unwrap();

        let loaded = loader.load();
        assert_eq!(loaded.len(), 2);

        // Update existing
        loader.add_command(UserCommand {
            name: "/first".to_string(),
            description: "Updated first".to_string(),
            action: "prompt".to_string(),
            prompt: "updated".to_string(),
        }).unwrap();

        let loaded = loader.load();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].description, "Updated first");
    }

    #[test]
    fn test_remove_command() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("commands.toml");
        let loader = CommandLoader::new(path);

        let commands = vec![
            UserCommand {
                name: "/keep".to_string(),
                description: "Keep".to_string(),
                action: "prompt".to_string(),
                prompt: "keep".to_string(),
            },
            UserCommand {
                name: "/remove".to_string(),
                description: "Remove".to_string(),
                action: "prompt".to_string(),
                prompt: "remove".to_string(),
            },
        ];
        loader.save(&commands).unwrap();

        let removed = loader.remove_command("/remove").unwrap();
        assert!(removed);

        let loaded = loader.load();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "/keep");

        let removed = loader.remove_command("/nonexistent").unwrap();
        assert!(!removed);
    }

    #[test]
    fn test_commands_section() {
        let builtin = vec![("/help", "Show help"), ("/models", "Switch model")];
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
