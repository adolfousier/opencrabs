//! Logging and Debug System
//!
//! Provides configurable logging with conditional file output for debug mode.

use std::path::PathBuf;
use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

/// Logging configuration
#[derive(Debug, Clone)]
pub struct LogConfig {
    /// Enable debug mode (creates log files)
    pub debug_mode: bool,

    /// Log directory path (default: .opencrabs/logs)
    pub log_dir: PathBuf,

    /// Minimum log level (default: INFO, DEBUG mode: DEBUG)
    pub log_level: Level,

    /// Enable console output (for non-TUI modes)
    pub console_output: bool,

    /// Log file name prefix
    pub log_prefix: String,

    /// Maximum log file age in days (for rotation)
    pub max_age_days: u64,
}

impl Default for LogConfig {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        Self {
            debug_mode: false,
            log_dir: home.join(".opencrabs").join("logs"),
            log_level: Level::INFO,
            console_output: false,
            log_prefix: "opencrabs".to_string(),
            max_age_days: 7,
        }
    }
}

impl LogConfig {
    /// Create a new log configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable debug mode (creates log files with DEBUG level)
    pub fn with_debug_mode(mut self, enabled: bool) -> Self {
        self.debug_mode = enabled;
        if enabled {
            self.log_level = Level::DEBUG;
        }
        self
    }

    /// Set custom log directory
    pub fn with_log_dir(mut self, dir: PathBuf) -> Self {
        self.log_dir = dir;
        self
    }

    /// Set log level
    pub fn with_log_level(mut self, level: Level) -> Self {
        self.log_level = level;
        self
    }

    /// Enable console output
    pub fn with_console_output(mut self, enabled: bool) -> Self {
        self.console_output = enabled;
        self
    }

    /// Set log file prefix
    pub fn with_log_prefix(mut self, prefix: String) -> Self {
        self.log_prefix = prefix;
        self
    }
}

/// Result of logger initialization
pub struct LoggerGuard {
    /// Keep the worker guard alive to ensure logs are flushed
    _guard: Option<WorkerGuard>,
}

impl LoggerGuard {
    /// Create a new guard (for debug mode with file logging)
    fn with_guard(guard: WorkerGuard) -> Self {
        Self {
            _guard: Some(guard),
        }
    }

    /// Create an empty guard (for non-debug mode)
    fn empty() -> Self {
        Self { _guard: None }
    }
}

/// Initialize the logging system
///
/// Returns a guard that must be kept alive for the duration of the program.
/// When the guard is dropped, logs are flushed.
///
/// # Arguments
/// * `config` - Logging configuration
///
/// # Behavior
/// - **Debug mode OFF**: No log files created, minimal console output
/// - **Debug mode ON**: Creates log files in `.opencrabs/logs/`, detailed logging
pub fn init_logging(config: LogConfig) -> Result<LoggerGuard, Box<dyn std::error::Error>> {
    if config.debug_mode {
        // Debug mode: Create log files in .opencrabs/logs/
        init_debug_logging(config)
    } else {
        // Normal mode: Minimal logging, no files
        init_minimal_logging(config)
    }
}

/// Initialize debug logging with file output
fn init_debug_logging(config: LogConfig) -> Result<LoggerGuard, Box<dyn std::error::Error>> {
    // Create log directory
    std::fs::create_dir_all(&config.log_dir)?;

    // Create gitignore file in .opencrabs to ignore logs
    let opencrabs_dir = config.log_dir.parent().unwrap_or(&config.log_dir);
    let gitignore_path = opencrabs_dir.join(".gitignore");
    if !gitignore_path.exists() {
        std::fs::write(
            &gitignore_path,
            "# Ignore all OpenCrabs runtime files\n*\n!.gitignore\n",
        )
        .ok();
    }

    // Set up rolling file appender (daily rotation)
    let file_appender = tracing_appender::rolling::daily(&config.log_dir, &config.log_prefix);
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // Build environment filter
    let env_filter = EnvFilter::from_default_env()
        .add_directive(config.log_level.into())
        .add_directive("sqlx=warn".parse()?)
        .add_directive("hyper=warn".parse()?)
        .add_directive("reqwest=warn".parse()?)
        .add_directive("tower=warn".parse()?)
        // whatsapp-rust logs TODO stubs for unimplemented upstream handlers — suppress
        .add_directive("whatsapp_rust::client=error".parse()?)
        .add_directive("whatsapp_rust=warn".parse()?);

    // Initialize subscriber with file logging
    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false) // No colors in log files
                .with_target(true)
                .with_thread_ids(true)
                .with_line_number(true)
                .with_file(true),
        )
        .init();

    // Log startup information
    tracing::info!("🚀 OpenCrabs debug mode enabled");
    tracing::info!("📁 Log directory: {}", config.log_dir.display());
    tracing::info!("📊 Log level: {:?}", config.log_level);
    tracing::debug!("Debug logging initialized successfully");

    Ok(LoggerGuard::with_guard(guard))
}

/// Initialize minimal logging (no file output)
fn init_minimal_logging(config: LogConfig) -> Result<LoggerGuard, Box<dyn std::error::Error>> {
    // Build environment filter - minimal logging
    let env_filter = EnvFilter::from_default_env()
        .add_directive(Level::WARN.into()) // Only warnings and errors
        .add_directive("opencrabs=info".parse()?); // INFO for opencrabs itself

    if config.console_output {
        // Console output for non-TUI modes
        tracing_subscriber::registry()
            .with(env_filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(std::io::stderr)
                    .with_ansi(true)
                    .with_target(false)
                    .compact(),
            )
            .init();
    } else {
        // Silent mode for TUI (no output to avoid interference)
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::sink))
            .init();
    }

    Ok(LoggerGuard::empty())
}

/// Convenience function to setup logging from CLI args
pub fn setup_from_cli(debug: bool) -> Result<LoggerGuard, Box<dyn std::error::Error>> {
    let config = LogConfig::new().with_debug_mode(debug);
    init_logging(config)
}

/// Get the path to the current log file (if debug mode is enabled)
pub fn get_log_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let log_dir = home.join(".opencrabs").join("logs");

    if log_dir.exists() {
        // Return the most recent log file
        std::fs::read_dir(&log_dir)
            .ok()?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .map(|ext| ext == "log")
                    .unwrap_or(false)
            })
            .max_by_key(|entry| entry.metadata().ok()?.modified().ok())
            .map(|entry| entry.path())
    } else {
        None
    }
}

/// Clean up old log files based on max age
pub fn cleanup_old_logs(max_age_days: u64) -> Result<usize, Box<dyn std::error::Error>> {
    let home = dirs::home_dir().ok_or("Could not determine home directory")?;
    let log_dir = home.join(".opencrabs").join("logs");

    if !log_dir.exists() {
        return Ok(0);
    }

    let max_age = std::time::Duration::from_secs(max_age_days * 24 * 60 * 60);
    let now = std::time::SystemTime::now();
    let mut removed = 0;

    for entry in std::fs::read_dir(&log_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().map(|ext| ext == "log").unwrap_or(false)
            && let Ok(metadata) = entry.metadata()
            && let Ok(modified) = metadata.modified()
            && let Ok(age) = now.duration_since(modified)
            && age > max_age
            && std::fs::remove_file(&path).is_ok()
        {
            removed += 1;
        }
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_config_default() {
        let config = LogConfig::default();
        assert!(!config.debug_mode);
        assert_eq!(config.log_level, Level::INFO);
        assert!(!config.console_output);
        assert_eq!(config.log_prefix, "opencrabs");
    }

    #[test]
    fn test_log_config_with_debug() {
        let config = LogConfig::new().with_debug_mode(true);
        assert!(config.debug_mode);
        assert_eq!(config.log_level, Level::DEBUG);
    }

    #[test]
    fn test_log_config_builder() {
        let config = LogConfig::new()
            .with_log_level(Level::TRACE)
            .with_console_output(true)
            .with_log_prefix("test".to_string());

        assert_eq!(config.log_level, Level::TRACE);
        assert!(config.console_output);
        assert_eq!(config.log_prefix, "test");
    }

    #[test]
    fn test_log_dir_in_home_opencrabs_folder() {
        let config = LogConfig::default();
        let log_dir_str = config.log_dir.to_string_lossy();
        assert!(log_dir_str.contains(".opencrabs"));
        assert!(log_dir_str.contains("logs"));
        // Should be under home dir, not cwd
        if let Some(home) = dirs::home_dir() {
            assert!(config.log_dir.starts_with(&home));
        }
    }
}
