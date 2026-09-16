//! Browser Manager
//!
//! Smart browser detection: finds the user's default/preferred Chromium-based
//! browser, connects to a running instance when possible, or launches a new one.
//! Manages named page sessions (tabs) for concurrent browsing.

use base64::Engine;
use chromiumoxide::browser::BrowserConfig;
use chromiumoxide::{Browser, Page};
use futures::StreamExt;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::config::BrowserConfig as ConfigBrowserConfig;

/// Result of closing a browser page (#739). Lets `browser_close` report the
/// truth instead of treating a failed close as "nothing to close".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseOutcome {
    /// A tab was open and the CDP close confirmed.
    Closed,
    /// No tab was open for this name — idempotent no-op.
    NothingOpen,
    /// A tab was open but the CDP close did not confirm (may already be gone).
    Failed,
}

/// Shared browser manager. Clone-safe via inner `Arc`.
#[derive(Clone)]
pub struct BrowserManager {
    inner: Arc<Mutex<ManagerInner>>,
    config: Arc<ConfigBrowserConfig>,
    /// CDP event ring (#1189). Held directly (its own std Mutex, no
    /// async lock) so sync drains from tool result paths never touch
    /// the tokio-guarded inner state.
    pub(crate) events: super::events::EventLog,
}

struct ManagerInner {
    browser: Option<Browser>,
    pages: HashMap<String, Page>,
    handler_handle: Option<tokio::task::JoinHandle<()>>,
    headless: bool,
    /// Per-session hash of the most recent screenshot bytes. Lets
    /// `browser_screenshot` reject a no-op repeat without sending another
    /// identical 25KB image into the LLM context. Updated on every
    /// successful screenshot; never explicitly invalidated — when the
    /// page genuinely changes the new bytes will hash differently.
    last_screenshot_hash: HashMap<uuid::Uuid, u64>,
    /// Per-session counter of consecutive identical screenshots. Increments
    /// when the hash matches the previous capture; resets on any page change
    /// (navigate, click, type, scroll). After 3 identical captures, forces
    /// a hard break to stop the screenshot loop (#664).
    consecutive_identical_screenshots: HashMap<uuid::Uuid, u32>,
}

/// Best-effort cleanup when the last `BrowserManager` clone (and
/// therefore the inner Arc) is dropped — typically app exit.
///
/// Previously, opencrabs shutdown left the CDP handler task running
/// as a tokio zombie and the headed Chrome process alive (visible
/// window, orphaned Arc<Browser>), since no signal handler or Drop
/// impl closed either. Subsequent opencrabs restarts would then hit
/// the SingletonLock path (fixed in P0) or fight over the user's
/// profile.
///
/// Drop can't be async, so we do the synchronous subset: abort the
/// handler task (JoinHandle::abort is sync and cancels the future at
/// the next await point), and drop the Browser option — Browser's own
/// Drop closes the CDP connection, which causes headless Chrome to
/// exit. Headed Chrome typically remains running visually until the
/// user closes it, which matches user expectations.
impl Drop for ManagerInner {
    fn drop(&mut self) {
        if self.browser.is_none() && self.handler_handle.is_none() {
            return; // never launched
        }
        self.pages.clear();
        self.browser.take();
        if let Some(h) = self.handler_handle.take() {
            h.abort();
        }
        tracing::debug!("browser: ManagerInner dropped — handler aborted, CDP closed");
    }
}

/// Normalize a user-configured CDP endpoint for `Browser::connect`.
///
/// chromey resolves the real `/devtools/browser/<id>` websocket URL via the
/// `/json/version` endpoint ONLY when the URL scheme is `http`/`https`. A bare
/// `ws://host:port` (no devtools path) is handed straight to the websocket
/// client, and Chromium rejects the upgrade on the root path — so the
/// documented `ws://localhost:9222` form would silently fail to connect.
///
/// This rewrites a path-less `ws://`/`wss://` endpoint to `http://`/`https://`
/// so resolution kicks in. A full devtools websocket URL (one that already has
/// a `/devtools/...` path) is left untouched, as is any `http(s)` endpoint.
pub(crate) fn normalize_cdp_endpoint(endpoint: &str) -> String {
    let trimmed = endpoint.trim().trim_end_matches('/');
    for (ws_scheme, http_scheme) in [("ws://", "http://"), ("wss://", "https://")] {
        if let Some(rest) = trimmed.strip_prefix(ws_scheme) {
            // Path-less host:port → rewrite scheme so /json/version resolution runs.
            // A real devtools URL contains a path and is left as-is.
            if !rest.contains('/') {
                return format!("{http_scheme}{rest}");
            }
            return endpoint.to_string();
        }
    }
    endpoint.to_string()
}

impl Default for BrowserManager {
    fn default() -> Self {
        Self::new(ConfigBrowserConfig::default())
    }
}

impl BrowserManager {
    pub fn new(config: ConfigBrowserConfig) -> Self {
        // Auto-detect: use headed mode only if a display is available
        let headless = !Self::has_display();
        if headless {
            tracing::info!("No display detected — browser will run headless");
        }
        Self::with_headless(headless, config)
    }

    /// Create a browser manager with explicit headless/headed mode.
    pub fn with_headless(headless: bool, config: ConfigBrowserConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ManagerInner {
                browser: None,
                pages: HashMap::new(),
                handler_handle: None,
                headless,
                last_screenshot_hash: HashMap::new(),
                consecutive_identical_screenshots: HashMap::new(),
            })),
            config: Arc::new(config),
            events: super::events::EventLog::default(),
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
        if let Some(handle) = inner.handler_handle.take() {
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

    /// Ensure the browser is launched. No-op if already running and the
    /// handler task is still alive.
    ///
    /// When the underlying Chrome process dies (OS kill, CDP socket
    /// break, OOM, user closed the window) the handler task completes
    /// and `handler_handle.is_finished()` becomes true. The Browser
    /// object is still in the map but its CDP channel is dead — the
    /// next `page.goto()` returns `Navigation failed: channel closed`
    /// (2026-04-19 09:49 log). We detect that here and relaunch.
    async fn ensure_browser(&self) -> anyhow::Result<()> {
        let mut inner = self.inner.lock().await;
        if inner.browser.is_some() && !handler_is_dead(inner.handler_handle.as_ref()) {
            return Ok(());
        }
        if inner.browser.is_some() {
            tracing::warn!(
                "browser: CDP handler task is dead — tearing down stale Browser handle and relaunching"
            );
            inner.pages.clear();
            inner.browser.take();
            if let Some(h) = inner.handler_handle.take() {
                h.abort();
            }
        }

        // If a CDP endpoint is configured, connect to it instead of launching
        // a new browser. This allows multiple profiles to share a single
        // Chromium instance, saving significant memory.
        if let Some(ref endpoint) = self.config.cdp_endpoint {
            // chromey's Browser::connect only auto-resolves the real
            // /devtools/browser/<id> websocket URL when the endpoint is an
            // http(s) URL (it queries /json/version). A bare `ws://host:port`
            // would be handed straight to the websocket client and Chromium
            // rejects the upgrade on the root path. Normalize a path-less
            // ws/wss endpoint to http/https so resolution kicks in — so both
            // `ws://localhost:9222` and `http://localhost:9222` work.
            let resolved = normalize_cdp_endpoint(endpoint);
            tracing::info!(
                "Connecting to existing Chromium instance at {endpoint} (resolved: {resolved})"
            );
            let (browser, mut handler) = Browser::connect(&resolved).await.map_err(|e| {
                anyhow::anyhow!(
                    "Failed to connect to CDP endpoint {endpoint} (resolved {resolved}): {e}"
                )
            })?;

            let handle = tokio::spawn(async move {
                while let Some(event) = handler.next().await {
                    if event.is_err() {
                        tracing::warn!("CDP handler error, browser connection may be lost");
                        break;
                    }
                }
            });

            inner.browser = Some(browser);
            inner.handler_handle = Some(handle);
            tracing::info!("Connected to external Chromium via CDP at {endpoint}");
            return Ok(());
        }

        let mode = if inner.headless { "headless" } else { "headed" };

        // Smart browser detection: default browser first, then any Chromium-based browser
        let detected = detect_browser();
        let browser_name = detected
            .as_ref()
            .map(|b| b.name.as_str())
            .unwrap_or("Chrome");
        let browser_path_for_log = detected
            .as_ref()
            .map(|b| b.path.display().to_string())
            .unwrap_or_else(|| "<chromey default>".to_string());
        tracing::info!("Launching {mode} {browser_name} via CDP (binary: {browser_path_for_log})");

        let mut builder = BrowserConfig::builder();
        // 1080p viewport (was 1280x720): the smaller window left pages half-cut
        // in the visible view and framed screenshots below the fold (#738).
        builder = builder.no_sandbox().window_size(1920, 1080);
        if !inner.headless {
            // Headed: maximize the OS window so the live view matches the
            // viewport instead of opening small and clipped (#738).
            builder = builder.with_head().arg("--start-maximized");
        }
        // Bump launch_timeout from chromey's default 20s to 60s. The
        // 2026-04-22 13:54 failure showed Brave on macOS occasionally
        // taking longer than 20s to print its `DevTools listening on
        // ws://...` line — 30s gap between two retries followed by a
        // BrowserStderr("") timeout. macOS app-bundle binaries can be
        // slow to print stderr when LaunchServices is in the loop, and
        // chromey's only wedge-recovery is the timeout. 60s gives slow
        // launches room without making real failures take noticeably
        // longer to surface.
        builder = builder.launch_timeout(std::time::Duration::from_secs(60));
        if let Some(ref info) = detected {
            builder = builder.chrome_executable(&info.path);
        }

        // Use the browser's own profile so the user's logins/cookies are available.
        // Falls back to our own profile dir if we can't find the browser's profile
        // or if it's locked by a running instance.
        //
        // Retry the lock check with exponential backoff before falling
        // through to the fallback. When the user's main browser is
        // *starting up*, Chrome briefly holds the profile lock before
        // releasing it for additional instances — falling straight
        // through on the first locked check drops the user into the
        // empty opencrabs fallback profile and silently loses all
        // their logins/cookies. Cap at ~10s total wait
        // (250ms → 500 → 1000 → 2000 → 4000 ≈ 7.75s; next retry
        // would exceed the cap so we stop).
        let native_profile = detected.as_ref().and_then(|b| b.user_data_dir.clone());
        let used_native;
        let profile_dir = match native_profile {
            Some(p) if p.exists() && wait_for_profile_unlock(&p, 10_000).await => {
                used_native = true;
                p
            }
            _ => {
                used_native = false;
                let fallback = crate::config::opencrabs_home().join("chrome-profile");
                if !fallback.exists() {
                    let _ = std::fs::create_dir_all(&fallback);
                }
                fallback
            }
        };

        // Sweep stale lock files in our OWN fallback profile before launch.
        // The 2026-04-11 / 17 logs all had the same failure: a previous
        // opencrabs Chrome process crashed, leaving `SingletonLock` behind
        // in `~/.opencrabs/chrome-profile`, and the next launch refused to
        // start with "Failed to create SingletonLock: File exists (17)".
        // Restricted to the opencrabs-owned fallback path — never touch
        // the user's native Chrome/Brave profile locks (that's their
        // running browser).
        let fallback_root = crate::config::opencrabs_home().join("chrome-profile");
        if profile_dir == fallback_root {
            clean_stale_locks(&profile_dir);
        }

        tracing::info!(
            "Browser profile: {} ({})",
            profile_dir.display(),
            if used_native {
                "native — has user logins/cookies"
            } else {
                "fallback — fresh, no user state"
            },
        );
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

        // Error message says the actual browser name we tried to launch
        // (Brave / Chrome / Edge / etc.). Hardcoding "Chrome" here was
        // confusing — users saw "Failed to launch Chrome" while we'd
        // actually been launching Brave, and assumed we'd ignored their
        // default browser entirely.
        let (browser, mut handler) = Browser::launch(config)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to launch {browser_name}: {e}"))?;

        let handle = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if event.is_err() {
                    tracing::warn!("CDP handler error, browser connection may be lost");
                    break;
                }
            }
        });

        inner.browser = Some(browser);
        inner.handler_handle = Some(handle);
        tracing::info!("{mode} {browser_name} launched successfully");
        Ok(())
    }

    /// Get or create a page keyed on an agent session id. Each agent
    /// session gets its own tab so concurrent turns on different
    /// sessions can't stomp on each other's DOM state (fixes P5 from
    /// the 2026-04-19 browser audit).
    pub async fn get_or_create_session_page(&self, session_id: uuid::Uuid) -> anyhow::Result<Page> {
        self.get_or_create_page(Some(&Self::page_name_for_session(session_id)))
            .await
    }

    /// Format a session id as the tab-name key the manager stores. The
    /// `session-` prefix is load-bearing — it keeps session tabs out
    /// of the old "default" namespace so a pre-P5 caller and a
    /// session-aware caller never collide on the same key.
    pub(crate) fn page_name_for_session(session_id: uuid::Uuid) -> String {
        format!("session-{}", session_id)
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

        // Register stealth patches via CDP's addScriptToEvaluateOnNewDocument
        // so they re-apply automatically on every navigation — including
        // cross-origin hops. Previously this was a one-shot `page.evaluate`
        // on page creation, so the patches survived same-document updates
        // but got reset on any full navigation (the JS context is
        // destroyed + rebuilt). Anti-bot scanners detect the half-patched
        // state; the new-document registration fixes this.
        Self::install_stealth_on_new_document(&page).await;

        // CDP event listeners (#1189): js dialogs, downloads, popups,
        // crashes, detachments feed the shared ring; every browser tool
        // result drains it as a `recent_events:` line.
        super::events::attach_page_listeners(&page, self.events.clone());

        inner.pages.insert(session_name, page.clone());
        Ok(page)
    }

    /// Register stealth JS via `Page.addScriptToEvaluateOnNewDocument`
    /// so Chrome runs it on every document creation — including full
    /// cross-origin navigations — without the caller needing to
    /// re-inject after each `goto`.
    async fn install_stealth_on_new_document(page: &Page) {
        if let Err(e) = page
            .add_script_to_evaluate_on_new_document(Some(STEALTH_JS.to_string()))
            .await
        {
            tracing::warn!("Stealth JS injection failed: {e}");
        }
    }

    /// Attach a screenshot to `result` for the given session's page.
    /// On success pushes the image and records `screenshot=ok` in the
    /// metadata. On failure appends a short explanatory note to the
    /// tool output and records `screenshot=failed` — the model used to
    /// see the primary text reply but had no way to know the visual
    /// it expected was missing.
    pub async fn attach_screenshot(
        &self,
        session_id: uuid::Uuid,
        result: &mut crate::brain::tools::ToolResult,
    ) {
        match self.take_screenshot_for_session(session_id).await {
            Some(img) => {
                result.images.push(img);
                result
                    .metadata
                    .insert("screenshot".to_string(), "ok".to_string());
            }
            None => {
                result
                    .output
                    .push_str("\n\n[screenshot unavailable — the page may not be rendered yet]");
                result
                    .metadata
                    .insert("screenshot".to_string(), "failed".to_string());
            }
        }
    }

    /// Take a screenshot of the session's page and return
    /// (media_type, base64_data). Never errors out — returns None if
    /// no page exists for the session or the capture fails.
    pub async fn take_screenshot_for_session(
        &self,
        session_id: uuid::Uuid,
    ) -> Option<(String, String)> {
        let key = Self::page_name_for_session(session_id);
        // Clone the Page handle while holding the lock briefly, then drop
        // the guard BEFORE awaiting the CDP screenshot call. Holding the
        // mutex across `.screenshot().await` blocks every other task that
        // needs the same mutex (handler events, page creation, page
        // close) for the entire round-trip — a deadlock hazard whenever
        // the CDP handler task wants to acquire the lock during the
        // screenshot. Page is an Arc-wrapped handle so the clone is
        // cheap.
        let page = {
            let inner = self.inner.lock().await;
            inner.pages.get(&key)?.clone()
        };
        let bytes = page
            .screenshot(
                chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotParams::builder()
                    .format(
                        chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png,
                    )
                    .build(),
            )
            .await
            .ok()?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Some(("image/png".to_string(), b64))
    }

    /// Legacy: take a screenshot of the hardcoded "default" page.
    /// Kept for any non-session-aware caller; session-aware tools
    /// should use `take_screenshot_for_session`.
    pub async fn take_screenshot(&self) -> Option<(String, String)> {
        // Same clone-then-drop-lock pattern as
        // `take_screenshot_for_session` to avoid holding the mutex across
        // the awaited CDP call.
        let page = {
            let inner = self.inner.lock().await;
            inner.pages.get("default")?.clone()
        };
        let bytes = page
            .screenshot(
                chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotParams::builder()
                    .format(
                        chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png,
                    )
                    .build(),
            )
            .await
            .ok()?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Some(("image/png".to_string(), b64))
    }

    /// Close a named page session and report a validated outcome (#739).
    ///
    /// Removes the Page handle from the HashMap AND issues
    /// `Target.closeTarget` over CDP so the actual Chrome tab closes —
    /// dropping the Arc-wrapped Page alone may not trigger the close on
    /// chromiumoxide's side. The handle is dropped from the map either way, so
    /// the next automation gets a fresh page even if the OS tab lingered —
    /// keeping transitions between many browser tasks smooth. Unlike the old
    /// bool (which returned true whenever an entry existed, hiding failures),
    /// this distinguishes closed / nothing-open / failed so `browser_close`
    /// can tell the agent the truth.
    pub async fn close_page(&self, name: &str) -> CloseOutcome {
        // Take the page out of the HashMap while holding the lock briefly.
        let removed = {
            let mut inner = self.inner.lock().await;
            inner.pages.remove(name)
        };
        let Some(page) = removed else {
            return CloseOutcome::NothingOpen;
        };
        match page.close().await {
            Ok(_) => CloseOutcome::Closed,
            Err(e) => {
                // The tab may already have been closed externally (still a
                // clean state for us), or the CDP call genuinely failed. Either
                // way our handle is gone; report Failed so the caller does not
                // claim a confirmed close.
                tracing::warn!(
                    "browser: CDP close_target failed for page '{name}' \
                     (tab may already be closed externally): {e}"
                );
                CloseOutcome::Failed
            }
        }
    }

    /// Close the page for a given agent session (#739).
    pub async fn close_page_for_session(&self, session_id: uuid::Uuid) -> CloseOutcome {
        self.close_page(&Self::page_name_for_session(session_id))
            .await
    }

    /// List active page session names.
    pub async fn list_pages(&self) -> Vec<String> {
        let inner = self.inner.lock().await;
        inner.pages.keys().cloned().collect()
    }

    /// Hash some screenshot bytes — used by `browser_screenshot` to detect
    /// no-op repeats. SipHash 1-3 (std DefaultHasher) is plenty for
    /// "are these two PNG byte vectors identical".
    pub fn hash_screenshot_bytes(bytes: &[u8]) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut h);
        h.finish()
    }

    /// Returns the previous hash for a session, if any. Lets the screenshot
    /// tool compare against the last capture and decide whether to short-
    /// circuit with a "page unchanged" error.
    pub async fn last_screenshot_hash(&self, session_id: uuid::Uuid) -> Option<u64> {
        self.inner
            .lock()
            .await
            .last_screenshot_hash
            .get(&session_id)
            .copied()
    }

    /// Record the hash of a screenshot for `session_id`. Subsequent calls
    /// to `last_screenshot_hash` return this value until a new capture
    /// overwrites it.
    pub async fn set_last_screenshot_hash(&self, session_id: uuid::Uuid, hash: u64) {
        self.inner
            .lock()
            .await
            .last_screenshot_hash
            .insert(session_id, hash);
    }

    /// Increment the consecutive-identical screenshot counter for a session.
    /// Returns the new count. Used to detect and break screenshot loops (#664).
    pub async fn increment_identical_screenshot_count(&self, session_id: uuid::Uuid) -> u32 {
        let mut inner = self.inner.lock().await;
        let count = inner
            .consecutive_identical_screenshots
            .entry(session_id)
            .or_insert(0);
        *count += 1;
        *count
    }

    /// Reset the consecutive-identical screenshot counter for a session.
    /// Called on any page change (navigate, click, type, scroll) to clear
    /// the loop detection state.
    pub async fn reset_identical_screenshot_count(&self, session_id: uuid::Uuid) {
        self.inner
            .lock()
            .await
            .consecutive_identical_screenshots
            .remove(&session_id);
    }

    /// Get the current consecutive-identical screenshot count for a session.
    /// Drain pending CDP events (#1189) formatted as the
    /// `recent_events: [...]` line, `None` when nothing fired since the
    /// last drain. Browser tools append this to their result text so
    /// spontaneous page activity (dialogs, downloads, crashes) surfaces
    /// exactly once, in the next result after it happened.
    pub fn drain_recent_events(&self) -> Option<String> {
        self.events.take_formatted()
    }

    /// Enumerate cross-origin (OOPIF) frame pages for the given session's
    /// main page (#1190). Chromium site-isolates cross-origin iframes into
    /// their own targets; chromey's auto-attach (flatten) already joins
    /// their sessions to the connection, so each frame is addressable as
    /// a `Page` via `Browser::get_page`.
    ///
    /// Returns `(frame_label, url, Page)` triples for every cross-origin
    /// child frame currently in the page's frame tree. Frames are matched
    /// to targets by URL (`type == "iframe"` targets carry the frame URL).
    /// Same-origin iframes are skipped — their DOM is reachable from the
    /// main frame's execution context, so the existing find/click JS
    /// already covers them.
    pub async fn oopif_pages(
        &self,
        session_id: uuid::Uuid,
    ) -> anyhow::Result<Vec<(String, String, Page)>> {
        use chromiumoxide::cdp::browser_protocol::page::GetFrameTreeParams;

        let page = self.get_or_create_session_page(session_id).await?;
        let tree = page
            .execute(GetFrameTreeParams::default())
            .await?
            .result
            .frame_tree;

        // Walk the tree collecting cross-origin children (any depth).
        let main_origin = origin_of(&tree.frame.url);
        let mut cross: Vec<(String, String)> = Vec::new();
        collect_cross_origin(&tree, &main_origin, &mut cross);

        if cross.is_empty() {
            return Ok(Vec::new());
        }

        // Map frame URLs to iframe targets, then to Pages.
        let mut inner = self.inner.lock().await;
        let browser = inner
            .browser
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Browser not initialized"))?;
        let targets = browser.fetch_targets().await?;
        let mut out = Vec::new();
        for (label, url) in cross {
            let target = targets
                .iter()
                .find(|t| t.r#type == "iframe" && t.url == url);
            if let Some(t) = target {
                match browser.get_page(t.target_id.clone()).await {
                    Ok(p) => out.push((label, url, p)),
                    Err(e) => {
                        tracing::debug!("browser: OOPIF page resolve failed for {url}: {e}")
                    }
                }
            }
        }
        Ok(out)
    }

    /// Resolve a single OOPIF page by its inventory label (`f1`, `f2`, …).
    /// `None` when the label doesn't match any current cross-origin frame
    /// (navigation changed the frame tree).
    pub async fn oopif_page_by_label(
        &self,
        session_id: uuid::Uuid,
        label: &str,
    ) -> anyhow::Result<Option<Page>> {
        Ok(self
            .oopif_pages(session_id)
            .await?
            .into_iter()
            .find(|(l, _, _)| l == label)
            .map(|(_, _, p)| p))
    }

    pub async fn identical_screenshot_count(&self, session_id: uuid::Uuid) -> u32 {
        self.inner
            .lock()
            .await
            .consecutive_identical_screenshots
            .get(&session_id)
            .copied()
            .unwrap_or(0)
    }

    /// Shut down the browser entirely.
    pub async fn shutdown(&self) {
        let mut inner = self.inner.lock().await;
        inner.pages.clear();
        inner.browser.take();
        if let Some(handle) = inner.handler_handle.take() {
            handle.abort();
        }
        tracing::info!("Browser shut down");
    }
}

/// Detected browser info.
pub(crate) struct BrowserInfo {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    /// The browser's native user-data directory (where cookies/logins live).
    user_data_dir: Option<PathBuf>,
}

/// All known Chromium-based browsers with their executable paths and profile dirs.
pub(crate) struct BrowserCandidate {
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
pub(crate) fn known_browsers() -> Vec<BrowserCandidate> {
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
pub(crate) fn is_profile_locked(profile_dir: &std::path::Path) -> bool {
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

/// Names of the lock artefacts Chrome writes under `--user-data-dir`.
/// Kept as a module-level constant so the test module can exercise the
/// exact same list.
pub(crate) const LOCK_FILES: &[&str] = &["SingletonLock", "SingletonSocket", "Lock"];

/// Stealth JavaScript registered via CDP's
/// `Page.addScriptToEvaluateOnNewDocument` so Chrome runs it on every
/// document creation — including full cross-origin navigations. Hides
/// the most common automation fingerprints: `navigator.webdriver`,
/// missing `chrome.runtime`, empty `navigator.plugins`, and the
/// notifications-permission probe used by bot detection libraries.
///
/// Exported `pub(crate)` for `src/tests/browser_stealth_test.rs` which
/// pins the presence of each patch as a regression guard.
pub(crate) const STEALTH_JS: &str = r#"
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

/// Remove stale Chrome singleton lock files from the OPENCRABS-OWNED
/// profile directory. A previous opencrabs Chrome process that crashed
/// leaves these files behind — the next launch then refuses to start
/// with "Failed to create SingletonLock: File exists (17)" (see
/// 2026-04-11 16:57 / 2026-04-17 15:00 logs).
///
/// This is safe to call unconditionally on our own profile: if a live
/// process genuinely holds the lock, the caller has already checked
/// `is_profile_locked` and is about to launch anyway — the OS-level
/// re-create will still be atomic. Restrict callers to paths that are
/// NEVER the user's native browser profile (doing this on a live
/// Chrome's profile would crash their main browser).
/// Decide whether the CDP handler task is dead and the cached Browser
/// handle should be discarded. Pure function over the handler's state
/// so `src/tests/browser_health_test.rs` can exercise both branches
/// without spawning a real Chrome.
///
/// Returns `true` when:
///   - the handler handle is `None` (never launched, or already torn down), or
///   - the handle exists but `is_finished()` — the task exited, usually
///     because the underlying Chrome process died (OS kill / OOM / user
///     closed the window / CDP socket break).
///
/// The production path calls this after the Browser Option is confirmed
/// Some, so `None` handle while browser is Some means "we forgot to
/// store the handle" — treat it as dead and relaunch (safer than
/// proceeding with an un-polled event stream).
pub(crate) fn handler_is_dead(handle: Option<&tokio::task::JoinHandle<()>>) -> bool {
    handle.map(|h| h.is_finished()).unwrap_or(true)
}

/// Poll `is_profile_locked` on `profile_dir` with exponential backoff
/// up to `cap_ms` total wait. Returns true once the profile becomes
/// unlocked, false if the cap elapses first.
///
/// When the user's main browser is starting up, Chrome briefly holds
/// the profile lock before releasing it for additional instances.
/// Checking once and falling straight through drops the user into the
/// empty fallback profile and silently loses all their
/// logins/cookies. Waiting a few seconds gets the native profile
/// back almost every time, at the cost of a small first-launch delay.
///
/// Delays: 250ms, 500, 1000, 2000, 4000 (capped at 4000ms per step).
pub(crate) async fn wait_for_profile_unlock(profile_dir: &std::path::Path, cap_ms: u64) -> bool {
    let mut waited_ms: u64 = 0;
    let mut delay_ms: u64 = 250;
    loop {
        if !is_profile_locked(profile_dir) {
            return true;
        }
        if waited_ms >= cap_ms {
            tracing::debug!(
                "browser: profile {} still locked after {}ms — caller falls back",
                profile_dir.display(),
                waited_ms
            );
            return false;
        }
        let step = delay_ms.min(cap_ms.saturating_sub(waited_ms));
        tracing::debug!(
            "browser: profile {} locked, retrying in {}ms",
            profile_dir.display(),
            step
        );
        tokio::time::sleep(std::time::Duration::from_millis(step)).await;
        waited_ms += step;
        delay_ms = (delay_ms * 2).min(4000);
    }
}

pub(crate) fn clean_stale_locks(profile_dir: &std::path::Path) {
    for name in LOCK_FILES {
        let path = profile_dir.join(name);
        if path.exists()
            && let Err(e) = std::fs::remove_file(&path)
        {
            tracing::warn!(
                "browser: failed to clean stale lock {}: {}",
                path.display(),
                e
            );
        }
    }
}

/// Detect the user's default browser (macOS).
///
/// Parses the `LSHandlers` array from LaunchServices. The plist output is
/// an array of dicts — we want the one with `LSHandlerURLScheme = https`
/// and read its `LSHandlerRoleAll` value.
///
/// The old parser walked the text line-by-line looking for a
/// `LSHandlerURLScheme = https` token and then grabbing the next
/// `LSHandlerRoleAll`. That was broken in two ways:
///
/// 1. Each entry starts with a nested `LSHandlerPreferredVersions` dict
///    whose contents include `LSHandlerRoleAll = "-";` as a placeholder.
///    The old parser grabbed that placeholder instead of the real role.
/// 2. Within each entry the role line appears BEFORE the scheme line,
///    so the `found_scheme` flag never fired on the correct role.
///
/// Result on this user's machine: the parser returned `"-"` as the
/// identifier, no candidate matched, detect_browser() fell through to
/// the first Chromium candidate (Google Chrome) — even when the user's
/// actual default browser is Brave. The fix is block-aware parsing:
/// track brace depth, accumulate fields at depth 2 only (skipping the
/// nested PreferredVersions dict at depth 3), and emit the pair
/// `(scheme, role)` when the block closes.
#[cfg(target_os = "macos")]
pub(crate) fn detect_default_browser_id() -> Option<String> {
    let output = std::process::Command::new("defaults")
        .args([
            "read",
            "com.apple.LaunchServices/com.apple.launchservices.secure",
            "LSHandlers",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    parse_ls_handlers(&text)
}

/// Pure parser for the `defaults read … LSHandlers` plist output.
/// Extracted so unit tests can feed fixtures without spawning `defaults`.
///
/// The `defaults` output wraps LSHandlers in `( … )` (plist array),
/// not `{ … }`. So each LSHandler dict entry `{ … }` opens at brace
/// depth 1, not 2. Fields at depth 1 are the entry's own keys; depth
/// 2 is inside a nested dict like LSHandlerPreferredVersions which we
/// skip. Entry closes when depth returns to 0.
///
/// We accept either `LSHandlerURLScheme = https` or
/// `LSHandlerContentType = "com.apple.default-app.web-browser"` as
/// the "default web browser" marker — System Settings → General →
/// Default web browser writes the content-type form; per-scheme
/// associations write the URL-scheme form.
#[cfg(target_os = "macos")]
pub(crate) fn parse_ls_handlers(text: &str) -> Option<String> {
    // Extract a quoted-or-unquoted value after `=` from a plist line.
    // Handles both `key = "value";` and `key = value;` forms.
    fn parse_value(line: &str) -> Option<String> {
        let eq = line.find('=')?;
        let rest = line[eq + 1..].trim().trim_end_matches(';').trim();
        let unquoted = rest.trim_matches('"').trim();
        if unquoted.is_empty() {
            None
        } else {
            Some(unquoted.to_string())
        }
    }

    let mut depth: i32 = 0;
    let mut block_scheme: Option<String> = None;
    let mut block_content_type: Option<String> = None;
    let mut block_role: Option<String> = None;

    for line in text.lines() {
        let trimmed = line.trim();

        if depth == 1 {
            if trimmed.starts_with("LSHandlerURLScheme") {
                block_scheme = parse_value(trimmed);
            } else if trimmed.starts_with("LSHandlerContentType") {
                block_content_type = parse_value(trimmed);
            } else if trimmed.starts_with("LSHandlerRoleAll") {
                block_role = parse_value(trimmed);
            }
        }

        depth += trimmed.matches('{').count() as i32;
        depth -= trimmed.matches('}').count() as i32;

        // Block boundary: returning to depth 0 means the entry closed.
        if depth == 0 {
            let scheme = block_scheme.take();
            let content_type = block_content_type.take();
            let role = block_role.take();
            let is_web_default = scheme.as_deref().map(|s| s.eq_ignore_ascii_case("https"))
                == Some(true)
                || content_type.as_deref() == Some("com.apple.default-app.web-browser");
            if is_web_default
                && let Some(r) = role
                && r != "-"
            {
                return Some(r.to_lowercase());
            }
        }
    }

    None
}

/// Detect the user's default browser (Linux).
#[cfg(target_os = "linux")]
pub(crate) fn detect_default_browser_id() -> Option<String> {
    let output = std::process::Command::new("xdg-settings")
        .args(["get", "default-web-browser"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    parse_xdg_default_browser(&text)
}

/// Pure parser for `xdg-settings get default-web-browser` output.
/// Extracted so unit tests can feed fixtures without spawning the
/// xdg-settings binary. Trims, lowercases, and rejects empty strings.
#[cfg(target_os = "linux")]
pub(crate) fn parse_xdg_default_browser(text: &str) -> Option<String> {
    let trimmed = text.trim().to_lowercase();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Detect the user's default browser (Windows).
#[cfg(target_os = "windows")]
pub(crate) fn detect_default_browser_id() -> Option<String> {
    let output = std::process::Command::new("reg")
        .args([
            "query",
            r"HKEY_CURRENT_USER\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice",
            "/v", "ProgId",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    parse_windows_reg_prog_id(&text)
}

/// Pure parser for `reg query … /v ProgId` output. Example input line:
/// `    ProgId    REG_SZ    ChromeHTML`. Returns the lowercased ProgId
/// value, or None if no ProgId line was found.
///
/// Extracted so unit tests can feed fixtures without shelling out to
/// `reg`. Matches the line containing "ProgId" and takes the last
/// whitespace-delimited token as the value — this works for the
/// standard `reg query` format on Windows 10/11.
#[cfg(target_os = "windows")]
pub(crate) fn parse_windows_reg_prog_id(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        // Skip the "ProgId" header line that some reg versions emit
        // without a value (shouldn't happen for REG_SZ queries, but
        // guard anyway).
        if trimmed.starts_with("ProgId") || trimmed.contains("    ProgId    ") {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            // Expect at least: ["ProgId", "REG_SZ", "<value>"]
            if parts.len() >= 3 {
                return Some(parts.last().unwrap().to_lowercase());
            }
        }
    }
    None
}

/// Case-insensitive equality check used by `matches_default` and the
/// associated tests. The actual comparison is split out so we can pin
/// it in a unit test without exposing the full `BrowserCandidate`
/// struct (which is platform-cfg-tangled). The 2026-04-19 macOS
/// fixtures showed `defaults` reporting `"com.brave.browser"`
/// (lowercase b) while our candidate carries `"com.brave.Browser"`
/// (capital B); without this case-insensitive compare we'd fall
/// through to Chrome.
pub(crate) fn id_matches_default(candidate_id: &str, default_id: &str) -> bool {
    candidate_id.eq_ignore_ascii_case(default_id)
}

/// Check if a browser candidate matches the detected default browser ID.
fn matches_default(candidate: &BrowserCandidate, default_id: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        id_matches_default(candidate.bundle_id, default_id)
    }
    #[cfg(target_os = "linux")]
    {
        id_matches_default(candidate.desktop_file, default_id)
    }
    #[cfg(target_os = "windows")]
    {
        id_matches_default(candidate.prog_id, default_id)
    }
}

/// Smart browser detection: finds the user's default browser, then falls back
/// to the first installed Chromium-based browser.
pub(crate) fn detect_browser() -> Option<BrowserInfo> {
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

/// Split a frame-routed selector (#1190): `f2:14` → `("f2", "14")`,
/// `f1:[data-opencrabs-match="3"]` → `("f1", "[data-opencrabs-match=\"3\"]")`.
/// Returns `None` for plain selectors (no `fN:` prefix). `f` must be
/// followed by digits and a colon; anything else is a normal selector —
/// so a CSS like `form:has(input)` stays untouched.
pub(crate) fn split_frame_selector(selector: &str) -> Option<(String, String)> {
    let rest = selector.strip_prefix('f')?;
    let digits_end = rest.find(':')?;
    let digits = &rest[..digits_end];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Reject `f1::x` (empty remainder) — a colon-only tail is not a
    // usable selector.
    let tail = &rest[digits_end + 1..];
    if tail.is_empty() {
        return None;
    }
    Some((format!("f{digits}"), tail.to_string()))
}

/// scheme://host:port of a URL, or the raw string when parsing fails
/// (about:blank and friends — such frames are never cross-origin in a
/// meaningful way, so treating them as their own origin is harmless).
pub(crate) fn origin_of(url: &str) -> String {
    url::Url::parse(url)
        .map(|u| match u.port() {
            Some(p) => format!("{}://{}:{}", u.scheme(), u.host_str().unwrap_or(""), p),
            None => format!("{}://{}", u.scheme(), u.host_str().unwrap_or("")),
        })
        .unwrap_or_else(|_| url.to_string())
}

/// Depth-first walk of the frame tree collecting cross-origin child
/// frames (relative to `main_origin`), labeled `f1`, `f2`, … in walk
/// order. Same-origin frames are skipped entirely — including their
/// subtrees' bookkeeping, since a cross-origin frame nested inside a
/// same-origin frame still gets its own entry.
pub(crate) fn collect_cross_origin(
    node: &chromiumoxide::cdp::browser_protocol::page::FrameTree,
    main_origin: &str,
    out: &mut Vec<(String, String)>,
) {
    if let Some(children) = &node.child_frames {
        for child in children {
            if origin_of(&child.frame.url) != main_origin {
                let label = format!("f{}", out.len() + 1);
                out.push((label, child.frame.url.clone()));
            }
            collect_cross_origin(child, main_origin, out);
        }
    }
}

/// Resolve a CSS selector to an element, entering shadow roots only when
/// the plain lookup fails.
///
/// `Page::find_element` issues `DOM.querySelector` rooted at the document
/// node, and CDP's querySelector does not cross a shadow boundary — the
/// `pierce: true` on the underlying `DOM.getDocument` buys nothing there.
/// So a selector stamped by `browser_find` inside a shadow root would
/// enumerate fine and then fail to click, which is strictly worse than
/// not enumerating it at all. This is the resolution half of that round
/// trip, shared by `browser_click`, `browser_wait`, `browser_screenshot`
/// and `browser_act`.
///
/// **Plain first, pierced only on failure.** A page with no shadow DOM
/// pays exactly zero extra round-trips, so there is no regression
/// surface on the common case; a shadow page pays one
/// `DOM.getDocument { depth: -1, pierce: true }` plus one query per tree.
///
/// This is also the only path that reaches a **closed** shadow root: JS
/// sees `el.shadowRoot === null` for one by spec, while `pierce: true`
/// returns it. Closed roots therefore resolve here but never appear in
/// `browser_find`'s inventory.
///
/// Uses `find_elements_pierced().remove(0)` rather than
/// `find_element_pierced()` because the latter is `els.pop()` — it hands
/// back the LAST match, where plain `find_element` hands back the first.
/// Taking the first keeps both branches of this fallback answering the
/// same question.
///
/// On failure the LIGHT-DOM error is returned, not the pierced one: it is
/// the one whose message names the selector the caller actually passed.
pub(crate) async fn resolve_element(
    page: &Page,
    selector: &str,
) -> std::result::Result<chromiumoxide::element::Element, chromiumoxide::error::CdpError> {
    match page.find_element(selector).await {
        Ok(el) => Ok(el),
        Err(light_err) => match page.find_elements_pierced(selector).await {
            Ok(mut els) if !els.is_empty() => Ok(els.remove(0)),
            _ => Err(light_err),
        },
    }
}
