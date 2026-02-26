//! CLI Module
//!
//! Command-line interface for OpenCrabs using Clap v4.

mod commands;
mod ui;

use anyhow::Result;
use clap::{Parser, Subcommand};

/// OpenCrabs - High-Performance Terminal AI Orchestration Agent
#[derive(Parser, Debug)]
#[command(name = "opencrabs")]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Enable debug mode (creates log files in .opencrabs/logs/)
    #[arg(short, long, global = true)]
    pub debug: bool,

    /// Configuration file path
    #[arg(short, long, global = true)]
    pub config: Option<String>,

    /// Subcommand to execute
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start interactive TUI mode (default)
    Chat {
        /// Session ID to resume
        #[arg(short, long)]
        session: Option<String>,

        /// Force onboarding wizard before chat
        #[arg(long)]
        onboard: bool,
    },

    /// Run the onboarding setup wizard
    Onboard,

    /// Run a single command non-interactively
    Run {
        /// The prompt to execute
        prompt: String,

        /// Auto-approve all tool executions (dangerous!)
        #[arg(long, alias = "yolo")]
        auto_approve: bool,

        /// Output format
        #[arg(short, long, default_value = "text")]
        format: OutputFormat,
    },

    /// Initialize configuration
    Init {
        /// Force overwrite existing configuration
        #[arg(short, long)]
        force: bool,
    },

    /// Show configuration
    Config {
        /// Show full configuration including secrets
        #[arg(short, long)]
        show_secrets: bool,
    },

    /// Database operations
    Db {
        #[command(subcommand)]
        operation: DbCommands,
    },

    /// Log management operations
    Logs {
        #[command(subcommand)]
        operation: LogCommands,
    },
}

#[derive(Subcommand, Debug)]
pub enum LogCommands {
    /// Show log file location and status
    Status,
    /// View recent log entries (requires debug mode)
    View {
        /// Number of lines to show (default: 50)
        #[arg(short, long, default_value = "50")]
        lines: usize,
    },
    /// Clean up old log files
    Clean {
        /// Maximum age in days (default: 7)
        #[arg(short = 'a', long, default_value = "7")]
        days: u64,
    },
    /// Open log directory in file manager
    Open,
}

#[derive(Subcommand, Debug)]
pub enum DbCommands {
    /// Initialize database
    Init,
    /// Show database statistics
    Stats,
    /// Clear all sessions and messages from database
    Clear {
        /// Skip confirmation prompt (use with caution)
        #[arg(short, long)]
        force: bool,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
    Markdown,
}

/// Main CLI entry point
pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    // Set up logging level based on debug flag
    if cli.debug {
        tracing::info!("Debug mode enabled");
    }

    // Load configuration
    let config = commands::load_config(cli.config.as_deref()).await?;

    // Auto-generate config.toml if API keys exist in env but no config file yet.
    // This prevents the onboarding wizard from triggering when .env is already set up.
    let config_path = dirs::config_dir().map(|d| d.join("opencrabs").join("config.toml"));
    if let Some(ref path) = config_path
        && !path.exists()
        && config.has_any_api_key()
    {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if let Err(e) = config.save(path) {
            tracing::warn!("Failed to auto-generate config.toml: {}", e);
        } else {
            tracing::info!("Auto-generated config.toml from environment");
        }
    }

    match cli.command {
        None | Some(Commands::Chat { .. }) => {
            // Default: Interactive TUI mode
            let (session, force_onboard) = match &cli.command {
                Some(Commands::Chat { session, onboard }) => (session.clone(), *onboard),
                _ => (None, false),
            };
            ui::cmd_chat(&config, session, force_onboard).await
        }
        Some(Commands::Onboard) => {
            // Launch TUI with onboarding wizard (skip splash)
            ui::cmd_chat(&config, None, true).await
        }
        Some(Commands::Init { force }) => commands::cmd_init(&config, force).await,
        Some(Commands::Config { show_secrets }) => {
            commands::cmd_config(&config, show_secrets).await
        }
        Some(Commands::Db { operation }) => commands::cmd_db(&config, operation).await,
        Some(Commands::Logs { operation }) => commands::cmd_logs(operation).await,
        Some(Commands::Run {
            prompt,
            auto_approve,
            format,
        }) => commands::cmd_run(&config, prompt, auto_approve, format).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parse() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
