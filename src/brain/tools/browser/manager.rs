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
}

impl Default for BrowserManager {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserManager {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ManagerInner {
                browser: None,
                pages: HashMap::new(),
                _handler_handle: None,
            })),
        }
    }

    /// Ensure the browser is launched. No-op if already running.
    async fn ensure_browser(&self) -> anyhow::Result<()> {
        let mut inner = self.inner.lock().await;
        if inner.browser.is_some() {
            return Ok(());
        }

        tracing::info!("Launching headless Chrome via CDP...");
        let config = BrowserConfig::builder()
            .no_sandbox()
            .window_size(1280, 720)
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
        tracing::info!("Headless Chrome launched successfully");
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

        inner.pages.insert(session_name, page.clone());
        Ok(page)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manager_new() {
        let mgr = BrowserManager::new();
        let _ = mgr.clone();
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
