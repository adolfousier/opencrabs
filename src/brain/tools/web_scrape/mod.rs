//! `web_scrape` — turn a URL into clean markdown at zero AI / zero API cost.
//!
//! When a user shares a link and asks the agent to check what's on it, the
//! cheap-and-correct path is: fetch the HTML, isolate the main content, strip
//! the structural junk (scripts, nav, footers, forms), and convert what's left
//! to markdown. The agent then reads a compact document instead of raw markup,
//! a 10x-30x token reduction with no model call to produce it. This is the
//! same idea as RTK for bash output, applied to the web.
//!
//! The pipeline is deliberately split into small, pure, single-responsibility
//! modules (ported and genericized from insight_forge's 4-year-in-production
//! scraper). Only [`fetch`] and [`sitemap`] touch the network; every other
//! stage is a pure `&str -> String` function, which keeps the `scraper` crate's
//! non-`Send` selector types off the async boundary (extraction runs to
//! completion synchronously, before any `.await`).
//!
//! Images survive as markdown `![alt](url)` references resolved to absolute
//! URLs, so the agent can see where images sit and vision only the specific
//! ones a task needs. Nothing here renders pages to screenshots or auto-OCRs
//! images: there is zero AI in the extraction path.

pub mod clean;
pub mod export;
pub mod extract;
pub mod fetch;
pub mod sitemap;
pub mod to_markdown;
pub mod tool;

pub use tool::WebScrapeTool;
