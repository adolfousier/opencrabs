//! Browser Manager
//!
//! Smart browser detection: finds the user's default/preferred Chromium-based
//! browser, connects to a running instance when possible, or launches a new one.
//! Manages named page sessions (tabs) for concurrent browsing.

use chromiumoxide::browser::BrowserConfig;
use chromiumoxide::{Browser, Page};
use futures::StreamExt;
use std::collections::HashMap;
use std::path::PathBuf;
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
    pub(crate) fn has_display() -> bool {
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

        // Smart browser detection: default browser first, then any Chromium-based browser
        let detected = detect_browser();
        let browser_name = detected
            .as_ref()
            .map(|b| b.name.as_str())
            .unwrap_or("Chrome");
        tracing::info!("Launching {mode} {browser_name} via CDP...");

        let mut builder = BrowserConfig::builder();
        builder = builder.no_sandbox().window_size(1280, 720);
        if !inner.headless {
            builder = builder.with_head();
        }
        if let Some(ref info) = detected {
            builder = builder.chrome_executable(&info.path);
            tracing::info!("Using browser: {} at {}", info.name, info.path.display());
        }

        // Use the browser's own profile so the user's logins/cookies are available.
        // Falls back to our own profile dir if we can't find the browser's profile
        // or if it's locked by a running instance.
        let profile_dir = detected
            .as_ref()
            .and_then(|b| b.user_data_dir.clone())
            .filter(|p| p.exists() && !is_profile_locked(p))
            .unwrap_or_else(|| {
                let fallback = crate::config::opencrabs_home().join("chrome-profile");
                if !fallback.exists() {
                    let _ = std::fs::create_dir_all(&fallback);
                }
                fallback
            });
        tracing::debug!("Browser profile: {}", profile_dir.display());
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
        tracing::info!("{mode} {browser_name} launched successfully");
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

/// Detected browser info.
struct BrowserInfo {
    name: String,
    path: PathBuf,
    /// The browser's native user-data directory (where cookies/logins live).
    user_data_dir: Option<PathBuf>,
}

/// All known Chromium-based browsers with their executable paths and profile dirs.
struct BrowserCandidate {
    name: &'static str,
    /// Bundle ID (macOS) or desktop file (Linux) or ProgId (Windows) for default detection.
    #[cfg(target_os = "macos")]
    bundle_id: &'static str,
    #[cfg(target_os = "linux")]
    desktop_file: &'static str,
    #[cfg(target_os = "windows")]
    prog_id: &'static str,
    paths: &'static [&'static str],
    /// PATH lookup names (e.g. "brave-browser", "google-chrome").
    which_names: &'static [&'static str],
    /// User data dir relative to platform config root.
    #[cfg(target_os = "macos")]
    profile_dir: Option<&'static str>,
    #[cfg(target_os = "linux")]
    profile_dir: Option<&'static str>,
    #[cfg(target_os = "windows")]
    profile_dir: Option<&'static str>,
}

/// Known Chromium-based browsers in preference order (most popular first).
fn known_browsers() -> Vec<BrowserCandidate> {
    vec![
        BrowserCandidate {
            name: "Google Chrome",
            #[cfg(target_os = "macos")]
            bundle_id: "com.google.chrome",
            #[cfg(target_os = "linux")]
            desktop_file: "google-chrome.desktop",
            #[cfg(target_os = "windows")]
            prog_id: "ChromeHTML",
            paths: if cfg!(target_os = "macos") {
                &["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"]
            } else if cfg!(target_os = "windows") {
                &[
                    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
                    r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
                ]
            } else {
                &["/usr/bin/google-chrome-stable", "/usr/bin/google-chrome"]
            },
            which_names: &["google-chrome-stable", "google-chrome"],
            #[cfg(target_os = "macos")]
            profile_dir: Some("Google/Chrome"),
            #[cfg(target_os = "linux")]
            profile_dir: Some("google-chrome"),
            #[cfg(target_os = "windows")]
            profile_dir: Some(r"Google\Chrome\User Data"),
        },
        BrowserCandidate {
            name: "Brave",
            #[cfg(target_os = "macos")]
            bundle_id: "com.brave.Browser",
            #[cfg(target_os = "linux")]
            desktop_file: "brave-browser.desktop",
            #[cfg(target_os = "windows")]
            prog_id: "BraveHTML",
            paths: if cfg!(target_os = "macos") {
                &["/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"]
            } else if cfg!(target_os = "windows") {
                &[r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe"]
            } else {
                &[
                    "/usr/bin/brave-browser",
                    "/usr/bin/brave",
                    "/opt/brave.com/brave/brave",
                ]
            },
            which_names: &["brave-browser", "brave"],
            #[cfg(target_os = "macos")]
            profile_dir: Some("BraveSoftware/Brave-Browser"),
            #[cfg(target_os = "linux")]
            profile_dir: Some("BraveSoftware/Brave-Browser"),
            #[cfg(target_os = "windows")]
            profile_dir: Some(r"BraveSoftware\Brave-Browser\User Data"),
        },
        BrowserCandidate {
            name: "Microsoft Edge",
            #[cfg(target_os = "macos")]
            bundle_id: "com.microsoft.edgemac",
            #[cfg(target_os = "linux")]
            desktop_file: "microsoft-edge.desktop",
            #[cfg(target_os = "windows")]
            prog_id: "MSEdgeHTM",
            paths: if cfg!(target_os = "macos") {
                &["/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"]
            } else if cfg!(target_os = "windows") {
                &[
                    r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
                    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
                ]
            } else {
                &["/usr/bin/microsoft-edge", "/opt/microsoft/msedge/msedge"]
            },
            which_names: &["microsoft-edge", "msedge"],
            #[cfg(target_os = "macos")]
            profile_dir: Some("Microsoft Edge"),
            #[cfg(target_os = "linux")]
            profile_dir: Some("microsoft-edge"),
            #[cfg(target_os = "windows")]
            profile_dir: Some(r"Microsoft\Edge\User Data"),
        },
        BrowserCandidate {
            name: "Arc",
            #[cfg(target_os = "macos")]
            bundle_id: "company.thebrowser.Browser",
            #[cfg(target_os = "linux")]
            desktop_file: "",
            #[cfg(target_os = "windows")]
            prog_id: "",
            paths: if cfg!(target_os = "macos") {
                &["/Applications/Arc.app/Contents/MacOS/Arc"]
            } else {
                &[]
            },
            which_names: &[],
            #[cfg(target_os = "macos")]
            profile_dir: Some("Arc/User Data"),
            #[cfg(target_os = "linux")]
            profile_dir: None,
            #[cfg(target_os = "windows")]
            profile_dir: None,
        },
        BrowserCandidate {
            name: "Vivaldi",
            #[cfg(target_os = "macos")]
            bundle_id: "com.vivaldi.Vivaldi",
            #[cfg(target_os = "linux")]
            desktop_file: "vivaldi-stable.desktop",
            #[cfg(target_os = "windows")]
            prog_id: "VivaldiHTM",
            paths: if cfg!(target_os = "macos") {
                &["/Applications/Vivaldi.app/Contents/MacOS/Vivaldi"]
            } else if cfg!(target_os = "windows") {
                &[r"C:\Program Files\Vivaldi\Application\vivaldi.exe"]
            } else {
                &["/usr/bin/vivaldi", "/opt/vivaldi/vivaldi"]
            },
            which_names: &["vivaldi"],
            #[cfg(target_os = "macos")]
            profile_dir: Some("Vivaldi"),
            #[cfg(target_os = "linux")]
            profile_dir: Some("vivaldi"),
            #[cfg(target_os = "windows")]
            profile_dir: Some(r"Vivaldi\User Data"),
        },
        BrowserCandidate {
            name: "Opera",
            #[cfg(target_os = "macos")]
            bundle_id: "com.operasoftware.Opera",
            #[cfg(target_os = "linux")]
            desktop_file: "opera.desktop",
            #[cfg(target_os = "windows")]
            prog_id: "OperaStable",
            paths: if cfg!(target_os = "macos") {
                &["/Applications/Opera.app/Contents/MacOS/Opera"]
            } else if cfg!(target_os = "windows") {
                &[r"C:\Program Files\Opera\launcher.exe"]
            } else {
                &["/usr/bin/opera"]
            },
            which_names: &["opera"],
            #[cfg(target_os = "macos")]
            profile_dir: Some("com.operasoftware.Opera"),
            #[cfg(target_os = "linux")]
            profile_dir: Some("opera"),
            #[cfg(target_os = "windows")]
            profile_dir: Some(r"Opera Software\Opera Stable"),
        },
        BrowserCandidate {
            name: "Chromium",
            #[cfg(target_os = "macos")]
            bundle_id: "org.chromium.Chromium",
            #[cfg(target_os = "linux")]
            desktop_file: "chromium-browser.desktop",
            #[cfg(target_os = "windows")]
            prog_id: "ChromiumHTM",
            paths: if cfg!(target_os = "macos") {
                &["/Applications/Chromium.app/Contents/MacOS/Chromium"]
            } else if cfg!(target_os = "windows") {
                &[r"C:\Program Files\Chromium\Application\chrome.exe"]
            } else {
                &["/usr/bin/chromium-browser", "/usr/bin/chromium"]
            },
            which_names: &["chromium-browser", "chromium"],
            #[cfg(target_os = "macos")]
            profile_dir: Some("Chromium"),
            #[cfg(target_os = "linux")]
            profile_dir: Some("chromium"),
            #[cfg(target_os = "windows")]
            profile_dir: Some(r"Chromium\User Data"),
        },
    ]
}

/// Find the executable path for a browser candidate.
fn find_executable(candidate: &BrowserCandidate) -> Option<PathBuf> {
    // Check known paths first
    for path in candidate.paths {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }
    // Fall back to PATH lookup
    for name in candidate.which_names {
        if let Ok(p) = which::which(name) {
            return Some(p);
        }
    }
    None
}

/// Resolve the browser's native user-data directory.
fn resolve_profile_dir(candidate: &BrowserCandidate) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let base = dirs::home_dir()?.join("Library/Application Support");
    #[cfg(target_os = "linux")]
    let base = dirs::config_dir()?;
    #[cfg(target_os = "windows")]
    let base = dirs::data_local_dir()?;

    let rel = candidate.profile_dir?;
    let dir = base.join(rel);
    if dir.exists() { Some(dir) } else { None }
}

/// Check if a profile directory is locked by a running browser instance.
fn is_profile_locked(profile_dir: &std::path::Path) -> bool {
    // Chrome-family browsers create a "SingletonLock" or "lockfile" when running
    let lock = profile_dir.join("SingletonLock");
    if lock.exists() {
        return true;
    }
    // Some browsers use "Lock" instead
    let lock2 = profile_dir.join("Lock");
    if lock2.exists() {
        return true;
    }
    // macOS: check for SingletonSocket too
    profile_dir.join("SingletonSocket").exists()
}

/// Detect the user's default browser (macOS).
#[cfg(target_os = "macos")]
fn detect_default_browser_id() -> Option<String> {
    let output = std::process::Command::new("defaults")
        .args([
            "read",
            "com.apple.LaunchServices/com.apple.launchservices.secure",
            "LSHandlers",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    // Parse the plist output looking for https handler
    let mut found_https = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.contains("LSHandlerURLScheme") && trimmed.contains("https") {
            found_https = true;
        }
        if found_https && trimmed.contains("LSHandlerRoleAll") {
            // Extract the bundle ID value
            if let Some(start) = trimmed.find('"')
                && let Some(end) = trimmed.rfind('"')
                && end > start
            {
                let id = &trimmed[start + 1..end];
                if !id.is_empty() {
                    return Some(id.to_lowercase());
                }
            }
            // Try without quotes (older format): LSHandlerRoleAll = "com.brave.Browser";
            if let Some(eq) = trimmed.find('=') {
                let val = trimmed[eq + 1..]
                    .trim()
                    .trim_matches(';')
                    .trim()
                    .trim_matches('"');
                if !val.is_empty() {
                    return Some(val.to_lowercase());
                }
            }
            found_https = false;
        }
    }
    None
}

/// Detect the user's default browser (Linux).
#[cfg(target_os = "linux")]
fn detect_default_browser_id() -> Option<String> {
    let output = std::process::Command::new("xdg-settings")
        .args(["get", "default-web-browser"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_lowercase();
    if text.is_empty() { None } else { Some(text) }
}

/// Detect the user's default browser (Windows).
#[cfg(target_os = "windows")]
fn detect_default_browser_id() -> Option<String> {
    let output = std::process::Command::new("reg")
        .args([
            "query",
            r"HKEY_CURRENT_USER\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice",
            "/v", "ProgId",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    // Output: "    ProgId    REG_SZ    ChromeHTML"
    for line in text.lines() {
        if line.contains("ProgId") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if let Some(id) = parts.last() {
                return Some(id.to_lowercase());
            }
        }
    }
    None
}

/// Check if a browser candidate matches the detected default browser ID.
fn matches_default(candidate: &BrowserCandidate, default_id: &str) -> bool {
    let id = default_id.to_lowercase();
    #[cfg(target_os = "macos")]
    {
        candidate.bundle_id.to_lowercase() == id
    }
    #[cfg(target_os = "linux")]
    {
        candidate.desktop_file.to_lowercase() == id
    }
    #[cfg(target_os = "windows")]
    {
        candidate.prog_id.to_lowercase() == id
    }
}

/// Smart browser detection: finds the user's default browser, then falls back
/// to the first installed Chromium-based browser.
fn detect_browser() -> Option<BrowserInfo> {
    let browsers = known_browsers();

    // 1. Try the user's default browser first
    if let Some(default_id) = detect_default_browser_id() {
        tracing::debug!("Default browser identifier: {default_id}");
        for candidate in &browsers {
            if matches_default(candidate, &default_id)
                && let Some(path) = find_executable(candidate)
            {
                tracing::info!(
                    "Default browser detected: {} ({})",
                    candidate.name,
                    default_id
                );
                return Some(BrowserInfo {
                    name: candidate.name.to_string(),
                    path,
                    user_data_dir: resolve_profile_dir(candidate),
                });
            }
        }
        tracing::debug!("Default browser '{default_id}' is not Chromium-based or not found");
    }

    // 2. Fall back to first installed Chromium browser
    for candidate in &browsers {
        if let Some(path) = find_executable(candidate) {
            tracing::info!("Found Chromium browser: {}", candidate.name);
            return Some(BrowserInfo {
                name: candidate.name.to_string(),
                path,
                user_data_dir: resolve_profile_dir(candidate),
            });
        }
    }

    tracing::warn!("No Chromium-based browser found on system");
    None
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

        if BrowserManager::has_display() {
            // Has display — switching to headed should succeed
            assert!(mgr.set_headless(false).await);
            assert!(!mgr.is_headless().await);
            // Switch back
            assert!(mgr.set_headless(true).await);
            assert!(mgr.is_headless().await);
        } else {
            // No display — switching to headed should be rejected, stays headless
            assert!(!mgr.set_headless(false).await);
            assert!(mgr.is_headless().await);
        }
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

    #[test]
    fn test_detect_browser_finds_something() {
        // On dev machines there should be at least one Chromium browser
        let result = detect_browser();
        if let Some(info) = result {
            assert!(!info.name.is_empty());
            assert!(info.path.exists());
            tracing::info!("Detected: {} at {}", info.name, info.path.display());
        }
        // On CI with no browser installed, None is acceptable
    }

    #[test]
    fn test_known_browsers_not_empty() {
        let browsers = known_browsers();
        assert!(browsers.len() >= 7); // Chrome, Brave, Edge, Arc, Vivaldi, Opera, Chromium
    }

    #[test]
    fn test_is_profile_locked_nonexistent() {
        let dir = std::path::PathBuf::from("/tmp/nonexistent-browser-profile-test");
        assert!(!is_profile_locked(&dir));
    }

    #[test]
    fn test_detect_default_browser_id() {
        // Just ensure it doesn't panic — result depends on system config
        let _id = detect_default_browser_id();
    }
}
