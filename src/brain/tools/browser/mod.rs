//! Browser automation tools — navigate, click, type, screenshot, eval JS, extract content.
//! Gated behind the `browser` feature flag.

mod act;
mod click;
mod close;
mod content;
mod eval;
pub(crate) mod events;
mod find;
pub(crate) mod manager;
mod navigate;
mod screenshot;
pub(crate) mod shadow;
mod type_text;
mod wait;

pub use act::BrowserActTool;
pub use click::BrowserClickTool;
pub use close::BrowserCloseTool;
pub use content::BrowserContentTool;
pub use eval::BrowserEvalTool;
pub use find::BrowserFindTool;

// Batched-action pure cores — re-exported only for test fixtures
// (src/tests/browser_act_test.rs).
#[cfg(test)]
pub(crate) use act::{Act, build_pre_flight_js, format_prefix_report, parse_actions};

// Eval output cap — re-exported only for test fixtures
// (src/tests/browser_eval_cap_test.rs).
#[cfg(test)]
pub(crate) use eval::cap_eval_output;

// Find-mode JS builder — re-exported only for test fixtures
// (src/tests/browser_find_test.rs).
#[cfg(test)]
pub(crate) use find::build_find_js;

// Inventory-mode JS builder — re-exported only for test fixtures
// (src/tests/browser_inventory_test.rs).
#[cfg(test)]
pub(crate) use find::build_inventory_js;
#[cfg(test)]
pub(crate) use find::inventory_header;

// Composed-tree JS helper builders — re-exported only for test fixtures
// (src/tests/browser_shadow_test.rs).
#[cfg(test)]
pub(crate) use shadow::{MAX_ROOTS, MAX_WALK_NODES, deep_helpers_js, with_deep_helpers};
pub use manager::{BrowserManager, CloseOutcome};
pub use navigate::BrowserNavigateTool;
pub use screenshot::BrowserScreenshotTool;
pub use type_text::BrowserTypeTool;
pub use wait::BrowserWaitTool;

// macOS LSHandlers plist parser — re-exported only for test fixtures
// (src/tests/browser_default_test.rs). Gated with `test` so clippy
// doesn't complain about it being unused in production builds.
#[cfg(all(target_os = "macos", test))]
pub(crate) use manager::parse_ls_handlers;

// Case-insensitive default-browser id comparison — re-exported for
// test fixtures (src/tests/browser_default_test.rs). The test file
// is gated `#![cfg(target_os = "macos")]`, so the re-export must
// match — otherwise Linux/Windows test builds see an unused import.
#[cfg(all(target_os = "macos", test))]
pub(crate) use manager::id_matches_default;

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
pub(crate) use manager::{LOCK_FILES, clean_stale_locks, wait_for_profile_unlock};

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
