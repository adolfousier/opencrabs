//! Tests for the Linux default-browser parser
//! (`parse_xdg_default_browser`).
//!
//! Gated to Linux — the parser itself is cfg'd to target_os = "linux"
//! since only the Linux detection path uses it.

#![cfg(all(feature = "browser", target_os = "linux"))]

use crate::brain::tools::browser::parse_xdg_default_browser;

#[test]
fn trims_trailing_newline_from_xdg_settings() {
    // `xdg-settings get default-web-browser` always emits a trailing
    // newline. The parser must strip it so the downstream
    // `matches_default` check sees `"firefox.desktop"` not
    // `"firefox.desktop\n"`.
    assert_eq!(
        parse_xdg_default_browser("firefox.desktop\n"),
        Some("firefox.desktop".into())
    );
}

#[test]
fn lowercases_the_result() {
    // `matches_default` lowercases both sides before comparing; having
    // the parser normalise up-front saves a step and avoids a subtle
    // bug if the candidate table adds a camel-case desktop file name.
    assert_eq!(
        parse_xdg_default_browser("Google-Chrome.Desktop\n"),
        Some("google-chrome.desktop".into())
    );
}

#[test]
fn rejects_empty_output() {
    // If xdg-settings isn't installed or returns nothing, we want
    // None (triggers fall-through to installed-browser scan) not
    // Some("").
    assert_eq!(parse_xdg_default_browser(""), None);
    assert_eq!(parse_xdg_default_browser("\n\n   \n"), None);
}

#[test]
fn handles_brave_browser_desktop() {
    // Realistic output on a user who set Brave as default via
    // `xdg-settings set default-web-browser brave-browser.desktop`.
    assert_eq!(
        parse_xdg_default_browser("brave-browser.desktop\n"),
        Some("brave-browser.desktop".into())
    );
}
