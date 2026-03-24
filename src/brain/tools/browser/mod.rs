//! Browser automation tools — navigate, click, type, screenshot, eval JS, extract content.
//! Gated behind the `browser` feature flag.

mod click;
mod content;
mod eval;
mod manager;
mod navigate;
mod screenshot;
mod type_text;
mod wait;

pub use click::BrowserClickTool;
pub use content::BrowserContentTool;
pub use eval::BrowserEvalTool;
pub use manager::BrowserManager;
pub use navigate::BrowserNavigateTool;
pub use screenshot::BrowserScreenshotTool;
pub use type_text::BrowserTypeTool;
pub use wait::BrowserWaitTool;
