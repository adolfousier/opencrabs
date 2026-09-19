//! Tests for `tui::error` — ErrorInfo creation, severity, category, and description.

use crate::tui::error::{ErrorCategory, ErrorInfo, ErrorSeverity};
use crate::tui::render::theme::{self, Role};

// ── ErrorSeverity ───────────────────────────────────────────────

#[test]
fn severity_prefix_not_empty() {
    for s in [
        ErrorSeverity::Info,
        ErrorSeverity::Warning,
        ErrorSeverity::Error,
        ErrorSeverity::Critical,
    ] {
        assert!(!s.prefix().is_empty(), "{:?} has empty prefix", s);
    }
}

#[test]
fn severity_name_matches() {
    assert_eq!(ErrorSeverity::Info.name(), "INFO");
    assert_eq!(ErrorSeverity::Warning.name(), "WARNING");
    assert_eq!(ErrorSeverity::Error.name(), "ERROR");
    assert_eq!(ErrorSeverity::Critical.name(), "CRITICAL");
}

#[test]
fn severity_colors_distinct() {
    let colors = [
        ErrorSeverity::Info.color(),
        ErrorSeverity::Warning.color(),
        ErrorSeverity::Error.color(),
        ErrorSeverity::Critical.color(),
    ];
    for (i, a) in colors.iter().enumerate() {
        for (j, b) in colors.iter().enumerate() {
            if i != j {
                assert_ne!(a, b, "severities {} and {} share color", i, j);
            }
        }
    }
}

// ── ErrorCategory ───────────────────────────────────────────────

#[test]
fn category_names() {
    assert_eq!(ErrorCategory::Network.name(), "Network");
    assert_eq!(ErrorCategory::Database.name(), "Database");
    assert_eq!(ErrorCategory::Config.name(), "Configuration");
    assert_eq!(ErrorCategory::Input.name(), "User Input");
    assert_eq!(ErrorCategory::Tool.name(), "Tool Execution");
    assert_eq!(ErrorCategory::Internal.name(), "Internal");
}

// ── ErrorInfo constructors ──────────────────────────────────────

#[test]
fn info_constructor() {
    let e = ErrorInfo::info("Test".to_string(), "msg".to_string());
    assert_eq!(e.severity, ErrorSeverity::Info);
    assert_eq!(e.category, ErrorCategory::Internal);
}

#[test]
fn warning_constructor() {
    let e = ErrorInfo::warning(
        ErrorCategory::Network,
        "Warn".to_string(),
        "msg".to_string(),
    );
    assert_eq!(e.severity, ErrorSeverity::Warning);
    assert_eq!(e.category, ErrorCategory::Network);
}

#[test]
fn error_constructor() {
    let e = ErrorInfo::error(
        ErrorCategory::Database,
        "Err".to_string(),
        "msg".to_string(),
    );
    assert_eq!(e.severity, ErrorSeverity::Error);
    assert_eq!(e.category, ErrorCategory::Database);
}

#[test]
fn critical_constructor() {
    let e = ErrorInfo::critical(ErrorCategory::Config, "Crit".to_string(), "msg".to_string());
    assert_eq!(e.severity, ErrorSeverity::Critical);
    assert_eq!(e.category, ErrorCategory::Config);
}

// ── ErrorInfo builder methods ───────────────────────────────────

#[test]
fn with_context_sets_context() {
    let e = ErrorInfo::error(ErrorCategory::Tool, "T".to_string(), "M".to_string())
        .with_context("extra info".to_string());
    assert_eq!(e.context.as_deref(), Some("extra info"));
}

#[test]
fn with_retry_sets_retryable_fields() {
    let next = chrono::Utc::now() + chrono::Duration::seconds(10);
    let e = ErrorInfo::error(ErrorCategory::Network, "T".to_string(), "M".to_string())
        .with_retry(3, next);
    assert!(e.is_retryable);
    assert_eq!(e.retry_count, Some(3));
    assert!(e.next_retry.is_some());
}

#[test]
fn default_not_retryable() {
    let e = ErrorInfo::error(ErrorCategory::Input, "T".to_string(), "M".to_string());
    assert!(!e.is_retryable);
    assert!(e.retry_count.is_none());
    assert!(e.next_retry.is_none());
}

// ── summary and description ─────────────────────────────────────

#[test]
fn summary_contains_category_and_title() {
    let e = ErrorInfo::warning(
        ErrorCategory::Database,
        "Slow Query".to_string(),
        "Took 5s".to_string(),
    );
    let s = e.summary();
    assert!(s.contains("Database"), "summary: {s}");
    assert!(s.contains("Slow Query"), "summary: {s}");
}

#[test]
fn description_contains_all_fields() {
    let e = ErrorInfo::error(
        ErrorCategory::Network,
        "Connection Failed".to_string(),
        "Could not connect".to_string(),
    )
    .with_context("retrying in 5s".to_string());
    let desc = e.description();
    assert!(desc.iter().any(|l| l.contains("ERROR")));
    assert!(desc.iter().any(|l| l.contains("Network")));
    assert!(desc.iter().any(|l| l.contains("Connection Failed")));
    assert!(desc.iter().any(|l| l.contains("Could not connect")));
    assert!(desc.iter().any(|l| l.contains("retrying in 5s")));
}

#[test]
fn description_includes_retry_info() {
    let next = chrono::Utc::now() + chrono::Duration::seconds(30);
    let e = ErrorInfo::error(ErrorCategory::Network, "T".to_string(), "M".to_string())
        .with_retry(2, next);
    let desc = e.description();
    assert!(desc.iter().any(|l| l.contains("Retry attempt: 2")));
    assert!(desc.iter().any(|l| l.contains("Next retry in:")));
}

// ── From impls ──────────────────────────────────────────────────

#[test]
fn from_string() {
    let e: ErrorInfo = "test error".to_string().into();
    assert_eq!(e.severity, ErrorSeverity::Error);
    assert_eq!(e.category, ErrorCategory::Internal);
    assert_eq!(e.message, "test error");
}

#[test]
fn from_str() {
    let e: ErrorInfo = "str error".into();
    assert_eq!(e.message, "str error");
    assert_eq!(e.title, "Error");
}

// ── recovered regression tests (from #248) ──────────────────────────────────

#[test]
fn test_error_severity_color() {
    // All four resolve through the theme. Error and Critical were the raw
    // Color::Red and Color::Magenta, ANSI indices the terminal resolved from
    // its own palette, so an error banner looked the same under every theme
    // (#1634). Asserted against `role` rather than literals so the test states
    // the contract (severity maps to a role) instead of re-pinning a constant.
    assert_eq!(ErrorSeverity::Info.color(), theme::role(Role::Gray));
    assert_eq!(ErrorSeverity::Warning.color(), theme::role(Role::Accent));
    assert_eq!(ErrorSeverity::Error.color(), theme::role(Role::Error));
    assert_eq!(
        ErrorSeverity::Critical.color(),
        theme::role(Role::PurpleSoft)
    );
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
    let next_retry = chrono::Utc::now() + chrono::Duration::seconds(30);
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
