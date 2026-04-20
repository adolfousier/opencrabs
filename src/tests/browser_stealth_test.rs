//! Regression guards for the stealth JS script registered via CDP's
//! `Page.addScriptToEvaluateOnNewDocument` on every new page.
//!
//! These tests don't launch a browser — they're pure content pins on
//! `STEALTH_JS` so accidentally removing one of the critical patches
//! (webdriver hiding, fake chrome.runtime, faked plugins/languages,
//! permissions shim) fails CI instead of silently becoming easier to
//! fingerprint.

#![cfg(feature = "browser")]

use crate::brain::tools::browser::STEALTH_JS;

#[test]
fn hides_navigator_webdriver() {
    assert!(
        STEALTH_JS.contains("navigator.webdriver") && STEALTH_JS.contains("get: () => undefined"),
        "STEALTH_JS must hide navigator.webdriver"
    );
}

#[test]
fn installs_fake_chrome_runtime() {
    assert!(
        STEALTH_JS.contains("window.chrome.runtime"),
        "STEALTH_JS must install a fake chrome.runtime"
    );
    assert!(
        STEALTH_JS.contains("sendMessage"),
        "STEALTH_JS must expose chrome.runtime.sendMessage"
    );
}

#[test]
fn installs_non_empty_plugins_array() {
    // Headless Chrome exposes an empty navigator.plugins by default —
    // every anti-bot library checks this. The stealth script must
    // forge at least one plugin entry.
    assert!(
        STEALTH_JS.contains("navigator, 'plugins'"),
        "STEALTH_JS must override navigator.plugins"
    );
    assert!(
        STEALTH_JS.contains("Chrome PDF Plugin"),
        "STEALTH_JS must include at least one named plugin"
    );
}

#[test]
fn installs_fake_languages() {
    assert!(
        STEALTH_JS.contains("navigator, 'languages'"),
        "STEALTH_JS must override navigator.languages"
    );
    assert!(
        STEALTH_JS.contains("'en-US'") || STEALTH_JS.contains("\"en-US\""),
        "STEALTH_JS must return a realistic language list"
    );
}

#[test]
fn installs_notifications_permission_shim() {
    // Headless Chrome reports `denied` for Notifications permission
    // but real browsers vary. Realistic scrapers match the live
    // `Notification.permission` value to blend in.
    assert!(
        STEALTH_JS.contains("permissions.query"),
        "STEALTH_JS must shim navigator.permissions.query"
    );
    assert!(
        STEALTH_JS.contains("Notification.permission"),
        "STEALTH_JS must mirror live Notification.permission for the notifications probe"
    );
}

#[test]
fn is_non_trivial_length() {
    // Sanity: accidentally clearing the constant to "" would silently
    // disable all patches. Require a realistic minimum size.
    assert!(
        STEALTH_JS.len() > 500,
        "STEALTH_JS looks suspiciously short ({} bytes) — did a patch get dropped?",
        STEALTH_JS.len()
    );
}
