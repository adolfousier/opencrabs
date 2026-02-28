//! Error Reporting for TUI
//!
//! Provides structured error information for display in the TUI.

use chrono::{DateTime, Utc};

/// Error severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorSeverity {
    /// Informational message (not really an error)
    Info,
    /// Warning that doesn't prevent operation
    Warning,
    /// Error that prevents current operation
    Error,
    /// Critical error requiring attention
    Critical,
}

impl ErrorSeverity {
    /// Get display color for this severity
    pub fn color(&self) -> ratatui::style::Color {
        use ratatui::style::Color;
        match self {
            ErrorSeverity::Info => Color::Rgb(120, 120, 120),
            ErrorSeverity::Warning => Color::Rgb(215, 100, 20),
            ErrorSeverity::Error => Color::Red,
            ErrorSeverity::Critical => Color::Magenta,
        }
    }

    /// Get display prefix for this severity
    pub fn prefix(&self) -> &'static str {
        match self {
            ErrorSeverity::Info => "ℹ️",
            ErrorSeverity::Warning => "⚠️",
            ErrorSeverity::Error => "❌",
            ErrorSeverity::Critical => "🔥",
        }
    }

    /// Get display name for this severity
    pub fn name(&self) -> &'static str {
        match self {
            ErrorSeverity::Info => "INFO",
            ErrorSeverity::Warning => "WARNING",
            ErrorSeverity::Error => "ERROR",
            ErrorSeverity::Critical => "CRITICAL",
        }
    }
}

/// Error category for classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    /// Network/API errors
    Network,
    /// Database errors
    Database,
    /// Configuration errors
    Config,
    /// User input errors
    Input,
    /// Tool execution errors
    Tool,
    /// Internal/unexpected errors
    Internal,
}

impl ErrorCategory {
    /// Get display name for this category
    pub fn name(&self) -> &'static str {
        match self {
            ErrorCategory::Network => "Network",
            ErrorCategory::Database => "Database",
            ErrorCategory::Config => "Configuration",
            ErrorCategory::Input => "User Input",
            ErrorCategory::Tool => "Tool Execution",
            ErrorCategory::Internal => "Internal",
        }
    }
}

/// Detailed error information for display
#[derive(Debug, Clone)]
pub struct ErrorInfo {
    /// Error severity
    pub severity: ErrorSeverity,
    /// Error category
    pub category: ErrorCategory,
    /// Short error title
    pub title: String,
    /// Detailed error message
    pub message: String,
    /// Optional context/additional information
    pub context: Option<String>,
    /// When the error occurred
    pub timestamp: DateTime<Utc>,
    /// Whether this error is retryable
    pub is_retryable: bool,
    /// Retry count if applicable
    pub retry_count: Option<u32>,
    /// Next retry time if applicable
    pub next_retry: Option<DateTime<Utc>>,
}

impl ErrorInfo {
    /// Create a new error info
    pub fn new(
        severity: ErrorSeverity,
        category: ErrorCategory,
        title: String,
        message: String,
    ) -> Self {
        Self {
            severity,
            category,
            title,
            message,
            context: None,
            timestamp: Utc::now(),
            is_retryable: false,
            retry_count: None,
            next_retry: None,
        }
    }

    /// Create an info-level error
    pub fn info(title: String, message: String) -> Self {
        Self::new(ErrorSeverity::Info, ErrorCategory::Internal, title, message)
    }

    /// Create a warning-level error
    pub fn warning(category: ErrorCategory, title: String, message: String) -> Self {
        Self::new(ErrorSeverity::Warning, category, title, message)
    }

    /// Create an error-level error
    pub fn error(category: ErrorCategory, title: String, message: String) -> Self {
        Self::new(ErrorSeverity::Error, category, title, message)
    }

    /// Create a critical-level error
    pub fn critical(category: ErrorCategory, title: String, message: String) -> Self {
        Self::new(ErrorSeverity::Critical, category, title, message)
    }

    /// Set additional context
    pub fn with_context(mut self, context: String) -> Self {
        self.context = Some(context);
        self
    }

    /// Mark as retryable with retry info
    pub fn with_retry(mut self, retry_count: u32, next_retry: DateTime<Utc>) -> Self {
        self.is_retryable = true;
        self.retry_count = Some(retry_count);
        self.next_retry = Some(next_retry);
        self
    }

    /// Get a short summary for status bar
    pub fn summary(&self) -> String {
        format!(
            "{} {}: {}",
            self.severity.prefix(),
            self.category.name(),
            self.title
        )
    }

    /// Get full description for error dialog
    pub fn description(&self) -> Vec<String> {
        let mut lines = vec![
            format!(
                "{}  {} - {}",
                self.severity.prefix(),
                self.severity.name(),
                self.category.name()
            ),
            String::new(),
            format!("Title: {}", self.title),
            format!("Time: {}", self.timestamp.format("%Y-%m-%d %H:%M:%S")),
            String::new(),
            "Message:".to_string(),
            self.message.clone(),
        ];

        if let Some(ref context) = self.context {
            lines.push(String::new());
            lines.push("Context:".to_string());
            lines.push(context.clone());
        }

        if self.is_retryable {
            lines.push(String::new());
            if let Some(count) = self.retry_count {
                lines.push(format!("Retry attempt: {}", count));
            }
            if let Some(next) = self.next_retry {
                let now = Utc::now();
                if next > now {
                    let duration = next - now;
                    lines.push(format!("Next retry in: {}s", duration.num_seconds()));
                }
            }
        }

        lines
    }
}

impl From<String> for ErrorInfo {
    fn from(message: String) -> Self {
        Self::error(ErrorCategory::Internal, "Error".to_string(), message)
    }
}

impl From<&str> for ErrorInfo {
    fn from(message: &str) -> Self {
        Self::error(
            ErrorCategory::Internal,
            "Error".to_string(),
            message.to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_severity_color() {
        use ratatui::style::Color;

        assert_eq!(ErrorSeverity::Info.color(), Color::Rgb(120, 120, 120));
        assert_eq!(ErrorSeverity::Warning.color(), Color::Rgb(215, 100, 20));
        assert_eq!(ErrorSeverity::Error.color(), Color::Red);
        assert_eq!(ErrorSeverity::Critical.color(), Color::Magenta);
    }

    #[test]
    fn test_error_info_creation() {
        let error = ErrorInfo::error(
            ErrorCategory::Network,
            "Connection Failed".to_string(),
            "Could not connect to server".to_string(),
        );

        assert_eq!(error.severity, ErrorSeverity::Error);
        assert_eq!(error.category, ErrorCategory::Network);
        assert_eq!(error.title, "Connection Failed");
        assert!(!error.is_retryable);
    }

    #[test]
    fn test_error_info_with_retry() {
        let next_retry = Utc::now() + chrono::Duration::seconds(30);
        let error = ErrorInfo::error(
            ErrorCategory::Network,
            "Rate Limited".to_string(),
            "Too many requests".to_string(),
        )
        .with_retry(2, next_retry);

        assert!(error.is_retryable);
        assert_eq!(error.retry_count, Some(2));
        assert!(error.next_retry.is_some());
    }

    #[test]
    fn test_error_info_summary() {
        let error = ErrorInfo::warning(
            ErrorCategory::Database,
            "Slow Query".to_string(),
            "Query took 5 seconds".to_string(),
        );

        let summary = error.summary();
        // Summary should contain database category and title
        assert!(summary.contains("Database"));
        assert!(summary.contains("Slow Query"));
    }

    #[test]
    fn test_error_info_from_string() {
        let error: ErrorInfo = "Test error".into();
        assert_eq!(error.severity, ErrorSeverity::Error);
        assert_eq!(error.message, "Test error");
    }
}
