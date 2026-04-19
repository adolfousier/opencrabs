//! Browser automation tools — navigate, click, type, screenshot, eval JS, extract content.
//! Gated behind the `browser` feature flag.

mod click;
mod content;
mod eval;
mod find;
mod manager;
mod navigate;
mod screenshot;
mod type_text;
mod wait;

pub use click::BrowserClickTool;
pub use content::BrowserContentTool;
pub use eval::BrowserEvalTool;
pub use find::BrowserFindTool;

// Eval output cap — re-exported only for test fixtures
// (src/tests/browser_eval_cap_test.rs).
#[cfg(test)]
pub(crate) use eval::cap_eval_output;

// Find-mode JS builder — re-exported only for test fixtures
// (src/tests/browser_find_test.rs).
#[cfg(test)]
pub(crate) use find::build_find_js;
pub use manager::BrowserManager;
pub use navigate::BrowserNavigateTool;
pub use screenshot::BrowserScreenshotTool;
pub use type_text::BrowserTypeTool;
pub use wait::BrowserWaitTool;

// macOS LSHandlers plist parser — re-exported only for test fixtures
// (src/tests/browser_default_test.rs). Gated with `test` so clippy
// doesn't complain about it being unused in production builds.
#[cfg(all(target_os = "macos", test))]
pub(crate) use manager::parse_ls_handlers;

// Linux xdg-settings parser — re-exported for tests.
#[cfg(all(target_os = "linux", test))]
pub(crate) use manager::parse_xdg_default_browser;

// Windows reg-query ProgId parser — re-exported for tests.
#[cfg(all(target_os = "windows", test))]
pub(crate) use manager::parse_windows_reg_prog_id;

// Stale-lock sweeper — re-exported only for test fixtures
// (src/tests/browser_locks_test.rs). See clean_stale_locks doc in
// manager.rs for the failure mode it guards against.
#[cfg(test)]
pub(crate) use manager::{clean_stale_locks, wait_for_profile_unlock, LOCK_FILES};

// CDP-handler-alive check — re-exported only for test fixtures
// (src/tests/browser_health_test.rs). See handler_is_dead doc in
// manager.rs.
#[cfg(test)]
pub(crate) use manager::handler_is_dead;

// Stealth JS source — re-exported only for test fixtures
// (src/tests/browser_stealth_test.rs) that pin the presence of each
// patch as a regression guard.
#[cfg(test)]
pub(crate) use manager::STEALTH_JS;

