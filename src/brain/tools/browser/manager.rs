//! Browser Manager
//!
//! Lazy-initialized singleton that launches headless Chrome on first use.
//! Manages named page sessions (tabs) for concurrent browsing.

use chromiumoxide::browser::BrowserConfig;
use chromiumoxide::{Browser, Page};
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Shared browser manager. Clone-safe via inner `Arc`.
#[derive(Clone)]
pub struct BrowserManager {
    inner: Arc<Mutex<ManagerInner>>,
}

struct ManagerInner {
    browser: Option<Browser>,
    pages: HashMap<String, Page>,
    _handler_handle: Option<tokio::task::JoinHandle<()>>,
    headless: bool,
}

impl Default for BrowserManager {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserManager {
    pub fn new() -> Self {
        // Auto-detect: use headed mode only if a display is available
        let headless = !Self::has_display();
        if headless {
            tracing::info!("No display detected — browser will run headless");
        }
        Self::with_headless(headless)
    }

    /// Create a browser manager with explicit headless/headed mode.
    pub fn with_headless(headless: bool) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ManagerInner {
                browser: None,
                pages: HashMap::new(),
                _handler_handle: None,
                headless,
            })),
        }
    }

    /// Detect whether a display server is available (X11, Wayland, or macOS/Windows).
    fn has_display() -> bool {
        if cfg!(target_os = "macos") || cfg!(target_os = "windows") {
            // macOS and Windows always have a display (unless headless server, rare)
            true
        } else {
            // Linux/Unix: check for DISPLAY (X11) or WAYLAND_DISPLAY
            std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok()
        }
    }

    /// Switch between headless and headed mode. Shuts down the current browser
    /// if the mode changes — the next page request will relaunch in the new mode.
    pub async fn set_headless(&self, headless: bool) -> bool {
        let mut inner = self.inner.lock().await;
        if inner.headless == headless {
            return false; // no change
        }
        // Prevent headed mode on headless environments (VPS without display)
        if !headless && !Self::has_display() {
            tracing::warn!("Cannot switch to headed mode — no display detected. Staying headless.");
            return false;
        }
        inner.headless = headless;
        // Tear down existing browser so it relaunches in the new mode
        inner.pages.clear();
        inner.browser.take();
        if let Some(handle) = inner._handler_handle.take() {
            handle.abort();
        }
        tracing::info!(
            "Browser mode switched to {}",
            if headless { "headless" } else { "headed" }
        );
        true
    }

    /// Returns the current headless mode.
    pub async fn is_headless(&self) -> bool {
        self.inner.lock().await.headless
    }

    /// Ensure the browser is launched. No-op if already running.
    async fn ensure_browser(&self) -> anyhow::Result<()> {
        let mut inner = self.inner.lock().await;
        if inner.browser.is_some() {
            return Ok(());
        }

        let mode = if inner.headless { "headless" } else { "headed" };
        tracing::info!("Launching {mode} Chrome via CDP...");
        let mut builder = BrowserConfig::builder();
        builder = builder.no_sandbox().window_size(1280, 720);
        if !inner.headless {
            builder = builder.with_head();
        }
        if let Some(path) = find_chrome_executable() {
            builder = builder.chrome_executable(path);
        }

        // Persistent profile — cookies, logins, and site preferences survive restarts
        let profile_dir = crate::config::opencrabs_home().join("chrome-profile");
        if !profile_dir.exists() {
            let _ = std::fs::create_dir_all(&profile_dir);
        }
        builder = builder.user_data_dir(profile_dir);

        // Stealth flags — reduce bot detection fingerprinting
        builder = builder
            .arg("--disable-blink-features=AutomationControlled")
            .arg("--disable-features=AutomationControlled")
            .arg("--disable-infobars")
            .arg("--disable-background-timer-throttling")
            .arg("--disable-backgrounding-occluded-windows")
            .arg("--disable-renderer-backgrounding")
            .arg("--disable-ipc-flooding-protection")
            .arg("--lang=en-US,en");

        let config = builder
            .build()
            .map_err(|e| anyhow::anyhow!("BrowserConfig error: {e}"))?;

        let (browser, mut handler) = Browser::launch(config)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to launch Chrome: {e}"))?;

        let handle = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if event.is_err() {
                    tracing::warn!("CDP handler error, browser connection may be lost");
                    break;
                }
            }
        });

        inner.browser = Some(browser);
        inner._handler_handle = Some(handle);
        tracing::info!("{mode} Chrome launched successfully");
        Ok(())
    }

    /// Get or create a named page (tab). Default name is "default".
    pub async fn get_or_create_page(&self, name: Option<&str>) -> anyhow::Result<Page> {
        self.ensure_browser().await?;
        let session_name = name.unwrap_or("default").to_string();

        let mut inner = self.inner.lock().await;
        if let Some(page) = inner.pages.get(&session_name) {
            return Ok(page.clone());
        }

        let browser = inner
            .browser
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Browser not initialized"))?;

        let page = browser
            .new_page("about:blank")
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create page: {e}"))?;

        // Inject stealth patches before any navigation
        Self::inject_stealth(&page).await;

        inner.pages.insert(session_name, page.clone());
        Ok(page)
    }

    /// Inject stealth JS to reduce bot detection fingerprinting.
    async fn inject_stealth(page: &Page) {
        let stealth_js = r#"
            // Hide navigator.webdriver
            Object.defineProperty(navigator, 'webdriver', { get: () => undefined });

            // Fake chrome.runtime (present in real Chrome, missing in automation)
            if (!window.chrome) { window.chrome = {}; }
            if (!window.chrome.runtime) {
                window.chrome.runtime = {
                    connect: function() {},
                    sendMessage: function() {},
                    id: undefined
                };
            }

            // Fake plugins array (headless has 0 plugins)
            Object.defineProperty(navigator, 'plugins', {
                get: () => [
                    { name: 'Chrome PDF Plugin', filename: 'internal-pdf-viewer' },
                    { name: 'Chrome PDF Viewer', filename: 'mhjfbmdgcfjbbpaeojofohoefgiehjai' },
                    { name: 'Native Client', filename: 'internal-nacl-plugin' }
                ]
            });

            // Fake languages
            Object.defineProperty(navigator, 'languages', {
                get: () => ['en-US', 'en']
            });

            // Remove automation-related properties from navigator
            const originalQuery = window.navigator.permissions.query;
            window.navigator.permissions.query = (parameters) =>
                parameters.name === 'notifications'
                    ? Promise.resolve({ state: Notification.permission })
                    : originalQuery(parameters);
        "#;

        if let Err(e) = page.evaluate(stealth_js).await {
            tracing::warn!("Stealth JS injection failed: {e}");
        }
    }

    /// Close a named page session.
    pub async fn close_page(&self, name: &str) -> bool {
        let mut inner = self.inner.lock().await;
        inner.pages.remove(name).is_some()
    }

    /// List active page session names.
    pub async fn list_pages(&self) -> Vec<String> {
        let inner = self.inner.lock().await;
        inner.pages.keys().cloned().collect()
    }

    /// Shut down the browser entirely.
    pub async fn shutdown(&self) {
        let mut inner = self.inner.lock().await;
        inner.pages.clear();
        inner.browser.take();
        if let Some(handle) = inner._handler_handle.take() {
            handle.abort();
        }
        tracing::info!("Browser shut down");
    }
}

/// Locate the Chrome/Chromium executable on the current platform.
fn find_chrome_executable() -> Option<std::path::PathBuf> {
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ]
    } else if cfg!(target_os = "windows") {
        &[
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        ]
    } else {
        // Linux — usually in PATH, but check common locations
        &[
            "/usr/bin/google-chrome-stable",
            "/usr/bin/google-chrome",
            "/usr/bin/chromium-browser",
            "/usr/bin/chromium",
        ]
    };

    for path in candidates {
        let p = std::path::PathBuf::from(path);
        if p.exists() {
            tracing::debug!("Found Chrome at {}", p.display());
            return Some(p);
        }
    }

    // Fall back to PATH lookup
    which::which("google-chrome")
        .or_else(|_| which::which("chromium"))
        .or_else(|_| which::which("chrome"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manager_new() {
        let mgr = BrowserManager::new();
        let _ = mgr.clone();
    }

    #[test]
    fn test_manager_with_headless() {
        let mgr = BrowserManager::with_headless(false);
        let _ = mgr.clone();
    }

    #[tokio::test]
    async fn test_is_headless_default() {
        let mgr = BrowserManager::with_headless(true);
        assert!(mgr.is_headless().await);
    }

    #[tokio::test]
    async fn test_is_headless_false() {
        let mgr = BrowserManager::with_headless(false);
        assert!(!mgr.is_headless().await);
    }

    #[tokio::test]
    async fn test_set_headless_no_change() {
        let mgr = BrowserManager::with_headless(true);
        // Already headless — no change
        assert!(!mgr.set_headless(true).await);
    }

    #[tokio::test]
    async fn test_set_headless_switch() {
        let mgr = BrowserManager::with_headless(true);
        assert!(mgr.is_headless().await);
        // Switch to headed
        assert!(mgr.set_headless(false).await);
        assert!(!mgr.is_headless().await);
        // Switch back
        assert!(mgr.set_headless(true).await);
        assert!(mgr.is_headless().await);
    }

    #[tokio::test]
    async fn test_list_pages_empty() {
        let mgr = BrowserManager::new();
        assert!(mgr.list_pages().await.is_empty());
    }

    #[tokio::test]
    async fn test_close_nonexistent() {
        let mgr = BrowserManager::new();
        assert!(!mgr.close_page("nonexistent").await);
    }
}
