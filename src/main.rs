use anyhow::Result;
use clap::Parser;
use opencrabs::{cli, logging};

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file before anything else (silently ignore if missing)
    dotenvy::dotenv().ok();

    // Parse CLI arguments first to check for debug flag
    let cli_args = cli::Cli::parse();

    // Initialize logging based on --debug flag
    let mut log_config = logging::LogConfig::new().with_debug_mode(cli_args.debug);

    // Custom log directory from env
    if let Ok(log_dir) = std::env::var("DEBUG_LOGS_LOCATION") {
        log_config = log_config.with_log_dir(std::path::PathBuf::from(log_dir));
    }

    let _guard = logging::init_logging(log_config)
        .map_err(|e| anyhow::anyhow!("Failed to initialize logging: {}", e))?;

    // Clean up old log files (keep last 7 days)
    if cli_args.debug
        && let Ok(removed) = logging::cleanup_old_logs(7)
            && removed > 0 {
                tracing::info!("🧹 Cleaned up {} old log file(s)", removed);
            }

    // Run CLI application
    cli::run().await
}
