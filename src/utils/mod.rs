//! Utility modules for common functionality

pub mod approval;
pub mod config_watcher;
pub mod file_extract;
pub mod image;
pub mod install;
pub mod pdf_vision;
pub mod providers;
pub mod retry;
pub mod sanitize;
pub mod slack_fmt;
mod string;
mod tool_context;

pub use approval::{
    check_approval_policy, persist_auto_always_policy, persist_auto_session_policy,
};
pub use file_extract::{FileContent, classify_file, inject_file_content, process_file_with_vision};
pub use image::extract_img_markers;
pub use retry::{RetryConfig, RetryableError, retry, retry_with_check};
pub use sanitize::redact_tool_input;
pub use string::truncate_str;
pub use tool_context::tool_context_hint;
