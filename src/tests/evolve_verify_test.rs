//! Release-asset integrity checks for the auto-updater (OC-03).
//!
//! The updater downloads a binary from GitHub and swaps it over the running
//! process. Before this, the only gate was a `--version` health probe, which a
//! hostile binary passes. These cover the host pin and the SHA256SUMS parse
//! that now gate the swap; both fail closed.

use crate::brain::tools::evolve::verify::{
    asset_basename, expected_digest, is_allowed_download_host,
};

// ── host pin ────────────────────────────────────────────────────────────────

#[test]
fn github_release_hosts_are_allowed() {
    assert!(is_allowed_download_host(
        "https://github.com/adolfousier/opencrabs/releases/download/v0.5.0/opencrabs-v0.5.0-aarch64-apple-darwin.tar.gz"
    ));
    assert!(is_allowed_download_host(
        "https://objects.githubusercontent.com/github-production-release-asset/x/y"
    ));
}

#[test]
fn any_other_host_is_refused() {
    assert!(!is_allowed_download_host(
        "https://example.invalid/opencrabs.tar.gz"
    ));
    assert!(!is_allowed_download_host(
        "https://evil.com/opencrabs.tar.gz"
    ));
    // A lookalike host must not pass on a suffix trick.
    assert!(!is_allowed_download_host("https://github.com.evil.com/x"));
    assert!(!is_allowed_download_host("https://notgithub.com/x"));
}

#[test]
fn plaintext_http_is_refused() {
    // TLS is what authenticates the host; an http asset URL is not trusted.
    assert!(!is_allowed_download_host(
        "http://github.com/adolfousier/opencrabs/releases/download/v0.5.0/x.tar.gz"
    ));
}

// ── SHA256SUMS parse ─────────────────────────────────────────────────────────

const SUMS: &str = "\
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  opencrabs-v0.5.0-x86_64-unknown-linux-gnu.tar.gz
bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  opencrabs-v0.5.0-aarch64-apple-darwin.tar.gz
";

#[test]
fn the_matching_asset_digest_is_returned() {
    assert_eq!(
        expected_digest(SUMS, "opencrabs-v0.5.0-aarch64-apple-darwin.tar.gz").as_deref(),
        Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    );
}

#[test]
fn a_missing_asset_has_no_digest() {
    // No entry ⇒ None ⇒ the caller refuses the swap.
    assert!(expected_digest(SUMS, "opencrabs-v0.5.0-windows.zip").is_none());
}

#[test]
fn a_binary_mode_star_prefix_is_tolerated() {
    let sums =
        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc *opencrabs-x.tar.gz\n";
    assert_eq!(
        expected_digest(sums, "opencrabs-x.tar.gz").as_deref(),
        Some("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc")
    );
}

#[test]
fn a_malformed_digest_is_rejected() {
    // Too short, or non-hex: not a usable checksum, so no match.
    let sums = "deadbeef  opencrabs-x.tar.gz\nnothex_nothex_nothex_nothex_nothex_nothex_nothex_nothex_nothe  opencrabs-y.tar.gz\n";
    assert!(expected_digest(sums, "opencrabs-x.tar.gz").is_none());
    assert!(expected_digest(sums, "opencrabs-y.tar.gz").is_none());
}

// ── asset basename ───────────────────────────────────────────────────────────

#[test]
fn the_asset_basename_is_the_url_filename() {
    assert_eq!(
        asset_basename("https://github.com/o/r/releases/download/v1/opencrabs-v1-linux.tar.gz")
            .as_deref(),
        Some("opencrabs-v1-linux.tar.gz")
    );
    assert!(asset_basename("https://github.com/o/r/releases/download/v1/").is_none());
}
