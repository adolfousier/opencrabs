//! Config hot-reload watcher.
//!
//! Watches `~/.opencrabs/config.toml` and `~/.opencrabs/keys.toml` for changes.
//! On any modification, re-loads the full `Config` and fires all registered callbacks.
//!
//! Designed to be extended: register any channel state update or command reload
//! by pushing a `ReloadCallback` via `spawn()`.

use crate::config::{Config, opencrabs_home};
use notify::{RecursiveMode, Watcher};
use std::sync::Arc;
use std::time::Duration;

/// Callback fired on every successful config reload.
pub type ReloadCallback = Arc<dyn Fn(Config) + Send + Sync>;

/// Spawn a background task that watches config files and fires callbacks on change.
/// Debounces rapid file-save events (300 ms window) before reloading.
///
/// # Example
/// ```ignore
/// config_watcher::spawn(vec![
///     Arc::new(move |cfg| {
///         let state = telegram_state.clone();
///         tokio::spawn(async move {
///             state.update_allowed_users(cfg.channels.telegram.allowed_users).await;
///         });
///     }),
/// ]);
/// ```
pub fn spawn(callbacks: Vec<ReloadCallback>) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Handle::current();
        let base = opencrabs_home();
        let config_path = base.join("config.toml");
        let keys_path = base.join("keys.toml");
        let commands_path = base.join("commands.toml");

        let (tx, rx) = std::sync::mpsc::channel();

        let mut watcher = match notify::recommended_watcher(move |res| {
            if let Ok(event) = res {
                let _ = tx.send(event);
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!("ConfigWatcher: failed to create watcher: {}", e);
                return;
            }
        };

        for path in [&config_path, &keys_path, &commands_path] {
            if path.exists()
                && let Err(e) = watcher.watch(path, RecursiveMode::NonRecursive)
            {
                tracing::warn!("ConfigWatcher: cannot watch {:?}: {}", path, e);
            }
        }

        tracing::info!(
            "ConfigWatcher: watching config.toml, keys.toml and commands.toml in {:?}",
            base
        );

        let debounce = Duration::from_millis(300);

        while rx.recv().is_ok() {
            // Drain further events within the debounce window
            let deadline = std::time::Instant::now() + debounce;
            loop {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    break;
                }
                match rx.recv_timeout(remaining) {
                    Ok(_) => {}
                    Err(_) => break,
                }
            }

            match Config::load() {
                Ok(new_config) => {
                    tracing::info!(
                        "ConfigWatcher: reloaded — firing {} callback(s)",
                        callbacks.len()
                    );
                    for cb in &callbacks {
                        let cb = cb.clone();
                        let cfg = new_config.clone();
                        rt.spawn(async move { cb(cfg) });
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "ConfigWatcher: reload failed, keeping current config: {}",
                        e
                    );
                }
            }
        }

        tracing::info!("ConfigWatcher: stopped");
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_reload_callback_fires_on_change() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        let keys_path = tmp.path().join("keys.toml");

        // Write initial files
        std::fs::write(&config_path, "[channels.telegram]\nenabled = false\n").unwrap();
        std::fs::write(&keys_path, "").unwrap();

        let call_count = Arc::new(AtomicUsize::new(0));
        let counter = call_count.clone();

        let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);

        let cb: ReloadCallback = Arc::new(move |_cfg| {
            counter.fetch_add(1, Ordering::Relaxed);
            let _ = tx.try_send(());
        });

        // Spawn watcher pointed at tmp dir files
        let _handle = {
            let config_path = config_path.clone();
            let keys_path = keys_path.clone();
            let callbacks = vec![cb];
            tokio::task::spawn_blocking(move || {
                let rt = tokio::runtime::Handle::current();
                let (tx, rx) = std::sync::mpsc::channel();
                let mut watcher = notify::recommended_watcher(move |res| {
                    if let Ok(event) = res {
                        let _ = tx.send(event);
                    }
                })
                .unwrap();
                let _ = watcher.watch(&config_path, notify::RecursiveMode::NonRecursive);
                let _ = watcher.watch(&keys_path, notify::RecursiveMode::NonRecursive);
                let debounce = std::time::Duration::from_millis(100);
                while rx.recv().is_ok() {
                    let deadline = std::time::Instant::now() + debounce;
                    loop {
                        let remaining =
                            deadline.saturating_duration_since(std::time::Instant::now());
                        if remaining.is_zero() {
                            break;
                        }
                        match rx.recv_timeout(remaining) {
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                    for cb in &callbacks {
                        let cb = cb.clone();
                        rt.spawn(async move { cb(crate::config::Config::default()) });
                    }
                }
            })
        };

        // Modify the file to trigger the watcher
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        std::fs::write(&config_path, "[channels.telegram]\nenabled = true\n").unwrap();

        // Wait up to 2s for callback to fire
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv()).await;
        assert!(
            result.is_ok(),
            "callback should have fired after file change"
        );
        assert!(call_count.load(Ordering::Relaxed) >= 1);
    }

    #[test]
    fn test_reload_callback_type_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ReloadCallback>();
    }
}
