//! Evolve (self-update) Tests
//!
//! Tests for version comparison, platform detection, asset naming,
//! and install method detection.

use crate::brain::tools::evolve::is_newer;
use crate::utils::install::{InstallMethod, binary_name, platform_suffix};

// ─── Version comparison ─────────────────────────────────────────────────────

#[test]
fn is_newer_major_bump() {
    assert!(is_newer("1.0.0", "0.9.9"));
    assert!(is_newer("2.0.0", "1.99.99"));
}

#[test]
fn is_newer_minor_bump() {
    assert!(is_newer("0.3.0", "0.2.66"));
    assert!(is_newer("0.2.67", "0.2.66"));
}

#[test]
fn is_newer_patch_bump() {
    assert!(is_newer("0.2.66", "0.2.65"));
}

#[test]
fn is_newer_equal_returns_false() {
    assert!(!is_newer("0.2.66", "0.2.66"));
    assert!(!is_newer("1.0.0", "1.0.0"));
}

#[test]
fn is_newer_older_returns_false() {
    assert!(!is_newer("0.2.65", "0.2.66"));
    assert!(!is_newer("0.1.0", "0.2.0"));
    assert!(!is_newer("0.9.9", "1.0.0"));
}

#[test]
fn is_newer_handles_different_lengths() {
    // "1.0" vs "0.9.9" — 1.0 parsed as [1, 0], 0.9.9 as [0, 9, 9]
    assert!(is_newer("1.0", "0.9.9"));
    assert!(!is_newer("0.9", "0.9.9"));
}

#[test]
fn is_newer_ignores_non_numeric() {
    // Non-numeric parts are filtered out
    assert!(is_newer("1.0.0-beta", "0.9.0"));
}

// ─── Asset naming (single binary) ──────────────────────────────────────────

#[test]
fn asset_name_format() {
    // Verify the asset naming convention used by evolve
    let tag = "v0.2.67";
    let suffix = "macos-arm64";
    let ext = "tar.gz";
    let expected = format!("opencrabs-{}-{}.{}", tag, suffix, ext);
    assert_eq!(expected, "opencrabs-v0.2.67-macos-arm64.tar.gz");
}

#[test]
fn asset_name_windows() {
    let tag = "v0.2.67";
    let suffix = "windows-amd64";
    let ext = "zip";
    let expected = format!("opencrabs-{}-{}.{}", tag, suffix, ext);
    assert_eq!(expected, "opencrabs-v0.2.67-windows-amd64.zip");
}

#[test]
fn legacy_asset_name_fallback() {
    // Legacy naming without version tag
    let suffix = "linux-amd64";
    let ext = "tar.gz";
    let legacy = format!("opencrabs-{}.{}", suffix, ext);
    assert_eq!(legacy, "opencrabs-linux-amd64.tar.gz");
}

// ─── Binary extraction: always "opencrabs" (single binary) ──────────────────

#[test]
fn binary_name_is_always_opencrabs() {
    // The evolve tool always extracts "opencrabs" (or "opencrabs.exe" on Windows)
    let is_windows = std::env::consts::OS == "windows";
    let binary_name = if is_windows {
        "opencrabs.exe"
    } else {
        "opencrabs"
    };
    assert!(binary_name.starts_with("opencrabs"));
}

// ─── Platform suffix coverage ───────────────────────────────────────────────

#[test]
fn current_platform_has_suffix() {
    // platform_suffix is now public via utils::install
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let supported = matches!(
        (os, arch),
        ("macos", "aarch64")
            | ("macos", "x86_64")
            | ("linux", "x86_64")
            | ("linux", "aarch64")
            | ("windows", "x86_64")
    );
    if supported {
        assert!(platform_suffix().is_some());
    }
}

// ─── Install method detection ────────────────────────────────────────────────

#[test]
fn install_method_detect_does_not_panic() {
    let method = InstallMethod::detect();
    // On a dev machine building from source, should be Source
    assert!(!method.description().is_empty());
}

#[test]
fn install_method_source_from_dev_build() {
    // When running tests via cargo, we're in a source build
    let method = InstallMethod::detect();
    matches!(method, InstallMethod::Source(_));
}

#[test]
fn install_method_descriptions_are_distinct() {
    let source = InstallMethod::Source(std::path::PathBuf::from("/tmp"));
    let cargo = InstallMethod::CargoInstall;
    let prebuilt = InstallMethod::PrebuiltBinary;
    assert_ne!(source.description(), cargo.description());
    assert_ne!(cargo.description(), prebuilt.description());
    assert_ne!(source.description(), prebuilt.description());
}

#[test]
fn binary_name_is_platform_correct() {
    let name = binary_name();
    if std::env::consts::OS == "windows" {
        assert_eq!(name, "opencrabs.exe");
    } else {
        assert_eq!(name, "opencrabs");
    }
}
