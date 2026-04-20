//! Tests for the Windows default-browser parser
//! (`parse_windows_reg_prog_id`).
//!
//! Gated to Windows — the parser is cfg'd to target_os = "windows"
//! since only the Windows detection path uses it.

#![cfg(all(feature = "browser", target_os = "windows"))]

use crate::brain::tools::browser::parse_windows_reg_prog_id;

/// Realistic `reg query … /v ProgId` output on Windows 10/11.
fn reg_output(prog_id: &str) -> String {
    format!(
        "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\Shell\\Associations\\UrlAssociations\\https\\UserChoice\r\n    ProgId    REG_SZ    {}\r\n\r\n",
        prog_id
    )
}

#[test]
fn resolves_chrome_html() {
    assert_eq!(
        parse_windows_reg_prog_id(&reg_output("ChromeHTML")),
        Some("chromehtml".into())
    );
}

#[test]
fn resolves_brave_html() {
    assert_eq!(
        parse_windows_reg_prog_id(&reg_output("BraveHTML")),
        Some("bravehtml".into())
    );
}

#[test]
fn resolves_msedge_html() {
    // Microsoft Edge's ProgId
    assert_eq!(
        parse_windows_reg_prog_id(&reg_output("MSEdgeHTM")),
        Some("msedgehtm".into())
    );
}

#[test]
fn returns_none_on_empty_output() {
    assert_eq!(parse_windows_reg_prog_id(""), None);
}

#[test]
fn returns_none_when_no_progid_line() {
    // `reg query` against a missing key prints an error on stderr and
    // nothing useful on stdout — our parser should return None.
    let stdout = "ERROR: The system was unable to find the specified registry key or value.\r\n";
    assert_eq!(parse_windows_reg_prog_id(stdout), None);
}

#[test]
fn output_is_lowercased() {
    // Same rationale as the Linux parser — matches_default compares
    // lowercased sides, so normalising up-front keeps that one-liner.
    assert_eq!(
        parse_windows_reg_prog_id(&reg_output("CHROMEHTML")),
        Some("chromehtml".into())
    );
}
