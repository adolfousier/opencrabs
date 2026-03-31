//! CLI argument types and entry point.

use anyhow::Result;
use clap::{Parser, Subcommand};

use super::{commands, cron, ui};

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

    /// Profile to use (default: "default", or OPENCRABS_PROFILE env)
    #[arg(short, long, global = true)]
    pub profile: Option<String>,

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

    /// Show system status: version, provider, channels, database, brain
    Status,

    /// Run diagnostics: check config, provider connectivity, channel health, tools, brain
    Doctor,

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

    /// Interactive CLI agent (no TUI) — multi-turn conversation in your terminal
    Agent {
        /// Single message mode (non-interactive)
        #[arg(short, long)]
        message: Option<String>,

        /// Session ID to resume
        #[arg(short, long)]
        session: Option<String>,

        /// Auto-approve all tool executions
        #[arg(long, alias = "yolo")]
        auto_approve: bool,

        /// Output format (only for single-message mode)
        #[arg(short, long, default_value = "text")]
        format: OutputFormat,
    },

    /// Channel operations
    Channel {
        #[command(subcommand)]
        operation: ChannelCommands,
    },

    /// Memory operations
    Memory {
        #[command(subcommand)]
        operation: MemoryCommands,
    },

    /// Session management
    Session {
        #[command(subcommand)]
        operation: SessionCommands,
    },

    /// OS service management (launchd/systemd)
    Service {
        #[command(subcommand)]
        operation: ServiceCommands,
    },

    /// Run in headless daemon mode — no TUI, channel bots only (Telegram, Discord, Slack, WhatsApp)
    /// Used by the systemd/LaunchAgent service installed during onboarding
    Daemon,

    /// Manage profiles — isolated OpenCrabs instances with their own config, DB, and memory
    Profile {
        #[command(subcommand)]
        operation: ProfileCommands,
    },

    /// Manage scheduled cron jobs
    Cron {
        #[command(subcommand)]
        operation: CronCommands,
    },

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Print version and exit
    Version,
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

#[derive(Subcommand, Debug)]
pub enum CronCommands {
    /// Add a new cron job
    Add {
        /// Job name
        #[arg(long)]
        name: String,

        /// Cron expression (5-field: min hour dom mon dow)
        #[arg(long)]
        cron: String,

        /// Timezone (default: UTC)
        #[arg(long, default_value = "UTC")]
        tz: String,

        /// Prompt / instructions for the agent
        #[arg(long, alias = "message")]
        prompt: String,

        /// Override provider (e.g. anthropic, openai)
        #[arg(long)]
        provider: Option<String>,

        /// Override model (e.g. claude-sonnet-4-20250514)
        #[arg(long)]
        model: Option<String>,

        /// Thinking mode: off, on, budget
        #[arg(long, default_value = "off")]
        thinking: String,

        /// Auto-approve tool executions
        #[arg(long, default_value = "true")]
        auto_approve: bool,

        /// Channel to deliver results (e.g. telegram:123456)
        #[arg(long, alias = "deliver")]
        deliver_to: Option<String>,
    },

    /// List all cron jobs
    List,

    /// Remove a cron job by ID or name
    Remove {
        /// Job ID or name
        id: String,
    },

    /// Enable a cron job
    Enable {
        /// Job ID or name
        id: String,
    },

    /// Disable a cron job (pause without deleting)
    Disable {
        /// Job ID or name
        id: String,
    },

    /// Trigger a cron job immediately (runs on next scheduler tick)
    Test {
        /// Job ID or name
        id: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum ChannelCommands {
    /// List configured channels and their status
    List,
    /// Run health checks on all enabled channels
    Doctor,
}

#[derive(Subcommand, Debug)]
pub enum MemoryCommands {
    /// List memory files in the brain directory
    List,
    /// Show a specific memory file
    Get {
        /// Memory file name (e.g. "MEMORY.md" or just "MEMORY")
        name: String,
    },
    /// Show memory statistics
    Stats,
}

#[derive(Subcommand, Debug)]
pub enum SessionCommands {
    /// List all sessions
    List {
        /// Include archived sessions
        #[arg(short, long)]
        all: bool,
    },
    /// Show session details
    Get {
        /// Session ID
        id: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum ProfileCommands {
    /// Create a new profile
    Create {
        /// Profile name (alphanumeric, hyphens, underscores)
        name: String,

        /// Optional description
        #[arg(short, long)]
        description: Option<String>,
    },
    /// List all profiles
    List,
    /// Delete a profile and all its data
    Delete {
        /// Profile name to delete
        name: String,

        /// Skip confirmation prompt
        #[arg(short, long)]
        force: bool,
    },
    /// Export a profile as a tar.gz archive
    Export {
        /// Profile name to export
        name: String,

        /// Output file path (default: <name>.tar.gz)
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Import a profile from a tar.gz archive
    Import {
        /// Path to the archive file
        path: String,
    },
    /// Migrate config and brain files from one profile to another (no DB/sessions)
    Migrate {
        /// Source profile name
        from: String,

        /// Destination profile name
        to: String,

        /// Overwrite existing files in the destination
        #[arg(short, long)]
        force: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum ServiceCommands {
    /// Install as OS service (launchd on macOS, systemd on Linux)
    Install,
    /// Start the service
    Start,
    /// Stop the service
    Stop,
    /// Restart the service
    Restart,
    /// Show service status
    Status,
    /// Uninstall the service
    Uninstall,
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

    // Set active profile BEFORE anything touches opencrabs_home()
    crate::config::profile::set_active_profile(cli.profile.clone())
        .unwrap_or_else(|e| tracing::warn!("Profile already set: {}", e));

    // Track profile usage
    if let Some(ref name) = cli.profile
        && let Ok(mut registry) = crate::config::profile::ProfileRegistry::load()
    {
        registry.touch(name);
        let _ = registry.save();
    }

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
        Some(Commands::Status) => commands::cmd_status(&config).await,
        Some(Commands::Doctor) => commands::cmd_doctor(&config).await,
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
        Some(Commands::Agent {
            message,
            session: _,
            auto_approve,
            format,
        }) => {
            if let Some(msg) = message {
                // Single message mode — same as `run`
                commands::cmd_run(&config, msg, auto_approve, format).await
            } else {
                // Interactive CLI agent (no TUI)
                commands::cmd_agent_interactive(&config, auto_approve).await
            }
        }
        Some(Commands::Channel { operation }) => commands::cmd_channel(&config, operation).await,
        Some(Commands::Memory { operation }) => commands::cmd_memory(operation).await,
        Some(Commands::Session { operation }) => commands::cmd_session(&config, operation).await,
        Some(Commands::Service { operation }) => commands::cmd_service(operation).await,
        Some(Commands::Daemon) => ui::cmd_daemon(&config).await,
        Some(Commands::Profile { operation }) => commands::cmd_profile(operation).await,
        Some(Commands::Cron { operation }) => cron::cmd_cron(&config, operation).await,
        Some(Commands::Completions { shell }) => {
            use clap::CommandFactory;
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "opencrabs",
                &mut std::io::stdout(),
            );
            Ok(())
        }
        Some(Commands::Version) => {
            println!("opencrabs {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}
