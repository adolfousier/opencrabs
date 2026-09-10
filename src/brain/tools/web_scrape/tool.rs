//! The `web_scrape` tool: the thin orchestrator that wires the pipeline modules
//! into a single agent-callable tool.
//!
//! It owns no scraping logic of its own. Each stage lives in its own module
//! ([`super::ssrf`], [`super::fetch`], [`super::extract`], [`super::clean`],
//! [`super::to_markdown`], [`super::sitemap`], [`super::export`]); this file
//! only sequences them and shapes the result.
//!
//! Modes:
//! - `readable` (default): fetch a page, isolate its main content, strip
//!   structural noise, and return clean markdown with images kept as
//!   `![alt](url)` references.
//! - `raw`: same, but skip main-content isolation and convert the whole page.
//! - `sitemap`: discover and crawl the site's sitemap into a URL list. With
//!   `export`, every page in the sitemap is scraped and written to disk.
//!
//! With `export: true`, the produced markdown is also written under the
//! session's project (or the active profile) via [`super::export`], so a scrape
//! can be captured to disk without the agent pasting the whole document back.

use super::super::error::{Result, ToolError};
use super::super::ssrf;
use super::super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use super::{clean, export, extract, fetch, sitemap, to_markdown};
use async_trait::async_trait;
use serde_json::{Value, json};
use uuid::Uuid;

/// Largest markdown payload returned inline. Exports write the full document to
/// disk regardless, so truncating the inline copy only bounds context, never
/// loses data.
const MAX_INLINE_BYTES: usize = 100_000;

/// Hard ceiling on pages scraped during a `sitemap` + `export` crawl, so a large
/// site can't fan a single call into thousands of fetches.
const SITEMAP_EXPORT_HARD_CAP: usize = 100;

/// Which extraction depth a page scrape uses.
#[derive(Clone, Copy)]
enum Mode {
    /// Isolate the main content container before converting.
    Readable,
    /// Convert the whole page after structural cleaning.
    Raw,
}

/// Native URL-to-markdown scraper. Holds an optional browser manager so it can
/// escalate JavaScript-only pages to a headless render; without the `browser`
/// feature it is a pure HTTP fetcher.
#[derive(Default)]
pub struct WebScrapeTool {
    #[cfg(feature = "browser")]
    browser: Option<std::sync::Arc<crate::brain::tools::browser::BrowserManager>>,
}

impl WebScrapeTool {
    /// Attach a browser manager for JS-shell render escalation.
    #[cfg(feature = "browser")]
    pub fn with_browser(
        mut self,
        manager: std::sync::Arc<crate::brain::tools::browser::BrowserManager>,
    ) -> Self {
        self.browser = Some(manager);
        self
    }

    /// Fetch a page's HTML, escalating to a headless render only when the static
    /// fetch looks like an unrendered JavaScript shell and a browser is wired.
    async fn fetch_html(
        &self,
        url: &str,
        session_id: Uuid,
        timeout: u64,
    ) -> std::result::Result<String, String> {
        let html = fetch::fetch_static(url, timeout).await?;

        #[cfg(feature = "browser")]
        if fetch::is_js_shell(&html)
            && let Some(browser) = &self.browser
        {
            match fetch::fetch_rendered(browser, session_id, url).await {
                Ok(rendered) => return Ok(rendered),
                Err(e) => tracing::debug!("web_scrape: render escalation failed for {url}: {e}"),
            }
        }
        #[cfg(not(feature = "browser"))]
        let _ = session_id;

        Ok(html)
    }

    /// Run the full page pipeline: SSRF-validate, fetch, isolate (readable) or
    /// keep (raw) the content, strip noise, absolutize URLs, convert to markdown.
    async fn scrape_markdown(
        &self,
        url: &str,
        mode: Mode,
        session_id: Uuid,
        timeout: u64,
    ) -> std::result::Result<String, String> {
        let base = ssrf::validate_url_resolved(url).await?;
        let html = self.fetch_html(url, session_id, timeout).await?;

        let content = match mode {
            Mode::Readable => extract::extract_main_content(&html),
            Mode::Raw => html,
        };
        let cleaned = clean::strip_noise(&content);
        let absolute = to_markdown::absolutize_urls(&cleaned, &base);
        let markdown = to_markdown::to_markdown(&absolute);
        Ok(clean::collapse_blank_lines(&markdown))
    }

    /// `sitemap` mode. Without `export`, return the discovered page-URL list for
    /// the agent to pick from. With `export`, scrape each page (bounded) and
    /// write the markdown to the session's export directory.
    async fn run_sitemap(
        &self,
        url: &str,
        export: bool,
        max_pages: usize,
        session_id: Uuid,
        timeout: u64,
        context: &ToolExecutionContext,
    ) -> Result<ToolResult> {
        // Root URL is SSRF-checked before we start touching robots.txt/sitemaps.
        if let Err(e) = ssrf::validate_url_resolved(url).await {
            return Ok(ToolResult::error(format!("web_scrape: {e}")));
        }

        // A URL that already points at a sitemap is used directly; otherwise we
        // auto-discover one from the site root.
        let sitemap_url = if url.to_ascii_lowercase().contains("sitemap") && url.ends_with(".xml") {
            url.to_string()
        } else {
            match sitemap::discover_sitemap_url(url, timeout).await {
                Some(s) => s,
                None => {
                    return Ok(ToolResult::error(format!(
                        "web_scrape: no sitemap found for {url} (tried /sitemap.xml, robots.txt, common alternates)"
                    )));
                }
            }
        };

        let pages = sitemap::collect_sitemap_urls(&sitemap_url, timeout).await;
        if pages.is_empty() {
            return Ok(ToolResult::error(format!(
                "web_scrape: sitemap {sitemap_url} yielded no page URLs"
            )));
        }

        if !export {
            let joined = pages.join("\n");
            let listing = truncate_utf8(&joined, MAX_INLINE_BYTES);
            return Ok(ToolResult::success(format!(
                "Found {} page URLs in {sitemap_url}:\n\n{listing}",
                pages.len()
            ))
            .with_metadata("sitemap_url".into(), sitemap_url)
            .with_metadata("url_count".into(), pages.len().to_string()));
        }

        // Export: scrape each page up to the requested cap and write to disk.
        let dir = export::resolve_export_dir(session_id, context.service_context.as_ref()).await;
        let limit = max_pages.min(SITEMAP_EXPORT_HARD_CAP).min(pages.len());
        let mut written = 0usize;
        let mut failed = 0usize;

        for page in pages.iter().take(limit) {
            match self
                .scrape_markdown(page, Mode::Readable, session_id, timeout)
                .await
            {
                Ok(md) => match export::write_markdown(&dir, page, &md).await {
                    Ok(_) => written += 1,
                    Err(e) => {
                        failed += 1;
                        tracing::debug!("web_scrape: export write failed for {page}: {e}");
                    }
                },
                Err(e) => {
                    failed += 1;
                    tracing::debug!("web_scrape: scrape failed for {page}: {e}");
                }
            }
        }

        let mut summary = format!(
            "Exported {written} of {} sitemap pages to {}",
            pages.len(),
            dir.display()
        );
        if limit < pages.len() {
            summary.push_str(&format!(
                "\n(capped at {limit} pages; raise max_pages up to {SITEMAP_EXPORT_HARD_CAP} for more)"
            ));
        }
        if failed > 0 {
            summary.push_str(&format!("\n{failed} page(s) failed and were skipped."));
        }

        Ok(ToolResult::success(summary)
            .with_metadata("export_dir".into(), dir.display().to_string())
            .with_metadata("pages_written".into(), written.to_string())
            .with_metadata("pages_total".into(), pages.len().to_string()))
    }
}

#[async_trait]
impl Tool for WebScrapeTool {
    fn name(&self) -> &str {
        "web_scrape"
    }

    fn description(&self) -> &str {
        "Fetch a URL and return clean markdown at zero AI / zero API cost. Isolates \
         the main content, strips scripts/nav/footers, and keeps images as \
         ![alt](url) references so you can vision only the ones a task needs. \
         Modes: 'readable' (default, main content only), 'raw' (whole page), \
         'sitemap' (list a site's page URLs, or with export=true scrape them all \
         to disk). Set export=true to also save markdown under the session's \
         project or profile directory. Prefer this over http_request for reading \
         page content."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The page URL to scrape, or a site root / sitemap URL in sitemap mode."
                },
                "mode": {
                    "type": "string",
                    "enum": ["readable", "raw", "sitemap"],
                    "description": "readable = main content as markdown (default); raw = whole page; sitemap = enumerate a site's page URLs.",
                    "default": "readable"
                },
                "export": {
                    "type": "boolean",
                    "description": "Also write the markdown to disk under the session's project (projects/<slug>/files/scrapes/) or active profile (~/.opencrabs[/profiles/<name>]/scrapes/). In sitemap mode, scrapes and saves every page.",
                    "default": false
                },
                "max_pages": {
                    "type": "integer",
                    "description": "sitemap + export only: cap on pages scraped (default 20, hard max 100).",
                    "default": 20
                }
            },
            "required": ["url"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        // Network only. Export writes are confined to the workspace scrapes
        // directory (never a user-supplied path), so this stays approval-free
        // like the other read-oriented web tools rather than gating every scrape.
        vec![ToolCapability::Network]
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let url = input
            .get("url")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::InvalidInput("web_scrape requires a 'url'".into()))?;

        let mode_str = input
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("readable");
        let export = input
            .get("export")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let max_pages = input
            .get("max_pages")
            .and_then(|v| v.as_u64())
            .unwrap_or(20) as usize;

        // Bound the per-request timeout so a slow site can't hang the agent.
        let timeout = context.timeout_secs.clamp(5, 120);
        let session_id = context.session_id;

        if mode_str == "sitemap" {
            return self
                .run_sitemap(url, export, max_pages, session_id, timeout, context)
                .await;
        }

        let mode = match mode_str {
            "raw" => Mode::Raw,
            _ => Mode::Readable,
        };

        let markdown = match self.scrape_markdown(url, mode, session_id, timeout).await {
            Ok(md) => md,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "web_scrape failed for {url}: {e}"
                )));
            }
        };

        let mut header = String::new();
        if export {
            let dir =
                export::resolve_export_dir(session_id, context.service_context.as_ref()).await;
            match export::write_markdown(&dir, url, &markdown).await {
                Ok(path) => header = format!("Saved to {}\n\n", path.display()),
                Err(e) => {
                    tracing::warn!("web_scrape: export write failed for {url}: {e}");
                    header = format!("(export failed: {e})\n\n");
                }
            }
        }

        let body = truncate_utf8(&markdown, MAX_INLINE_BYTES);
        let truncated = markdown.len() > body.len();
        let mut output = format!("{header}{body}");
        if truncated {
            output.push_str(&format!(
                "\n\n[truncated to {} of {} bytes{}]",
                body.len(),
                markdown.len(),
                if export {
                    "; full document written to disk"
                } else {
                    ""
                }
            ));
        }

        Ok(ToolResult::success(output)
            .with_metadata("url".into(), url.to_string())
            .with_metadata("bytes".into(), markdown.len().to_string()))
    }
}

/// Truncate `s` to at most `max_bytes` on a UTF-8 char boundary.
fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    crate::utils::string::truncate_str(s, max_bytes)
}
