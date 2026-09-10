//! Integrity verification for downloaded release assets (OC-03).
//!
//! Every release publishes a `SHA256SUMS` asset (`.github/workflows/release.yml`
//! runs `sha256sum *.tar.gz *.zip > SHA256SUMS`). The updater must verify the
//! downloaded archive against it before swapping the running binary, or a
//! compromised release, a stolen release token, or a hijacked
//! `browser_download_url` becomes code execution on the next auto-update. The
//! only prior check was a `--version` health probe, which a hostile binary
//! passes trivially.
//!
//! This fails closed: a missing SHA256SUMS asset, a missing entry for the
//! asset, a download served from an unexpected host, or a digest mismatch all
//! refuse the swap.

use serde_json::Value;
use sha2::{Digest, Sha256};

/// Hosts a release asset may legitimately be served from. GitHub browser
/// download URLs live on github.com and redirect to
/// objects.githubusercontent.com; pinning to these stops a release JSON whose
/// `browser_download_url` points off-GitHub from being fetched at all.
const ALLOWED_HOSTS: &[&str] = &["github.com", "objects.githubusercontent.com"];

/// True when `url` is an https URL whose host we trust to serve a release asset.
pub(crate) fn is_allowed_download_host(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "https" {
        return false;
    }
    parsed.host_str().is_some_and(|h| {
        ALLOWED_HOSTS
            .iter()
            .any(|a| h == *a || h.ends_with(&format!(".{a}")))
    })
}

/// The `SHA256SUMS` asset's download URL, if the release publishes one.
fn sha256sums_url(release: &Value) -> Option<String> {
    release["assets"].as_array()?.iter().find_map(|a| {
        if a["name"].as_str()? == "SHA256SUMS" {
            a["browser_download_url"].as_str().map(String::from)
        } else {
            None
        }
    })
}

/// Lowercase hex SHA-256 of `bytes`.
fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// The expected digest for `asset_name` from a `sha256sum`-format document, or
/// None when the file has no entry for it. Lines are `<64 hex>  <filename>`;
/// the filename may carry a leading `*` in binary mode.
pub(crate) fn expected_digest(sums_text: &str, asset_name: &str) -> Option<String> {
    for line in sums_text.lines() {
        let mut parts = line.split_whitespace();
        let hex = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        if hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) && name == asset_name {
            return Some(hex.to_ascii_lowercase());
        }
    }
    None
}

/// Verify `archive_bytes` against the release's SHA256SUMS entry for
/// `asset_name`. Fails closed on every uncertainty.
pub(super) async fn verify_asset_checksum(
    client: &reqwest::Client,
    release: &Value,
    asset_name: &str,
    archive_bytes: &[u8],
) -> Result<(), String> {
    let sums_url = sha256sums_url(release).ok_or_else(|| {
        "release publishes no SHA256SUMS asset; refusing to swap an unverified binary (OC-03)"
            .to_string()
    })?;
    if !is_allowed_download_host(&sums_url) {
        return Err(format!(
            "SHA256SUMS is served from an unexpected host, refusing: {sums_url}"
        ));
    }

    let sums_text = client
        .get(&sums_url)
        .send()
        .await
        .map_err(|e| format!("failed to fetch SHA256SUMS: {e}"))?
        .error_for_status()
        .map_err(|e| format!("SHA256SUMS fetch returned an error status: {e}"))?
        .text()
        .await
        .map_err(|e| format!("failed to read SHA256SUMS body: {e}"))?;

    let expected = expected_digest(&sums_text, asset_name).ok_or_else(|| {
        format!("SHA256SUMS has no entry for {asset_name}; refusing to swap (OC-03)")
    })?;

    let actual = hex_sha256(archive_bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "checksum mismatch for {asset_name}: SHA256SUMS says {expected}, downloaded asset is \
             {actual}; refusing to swap (OC-03)"
        ))
    }
}

/// The filename portion of a release download URL, used to match its
/// SHA256SUMS line.
pub(crate) fn asset_basename(url: &str) -> Option<String> {
    url.rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// True when the current process runs as root (euid 0). Unix only; false
/// elsewhere, where this specific root-swap concern does not arise.
pub(super) fn running_as_root() -> bool {
    #[cfg(unix)]
    {
        // SAFETY: geteuid() always succeeds and has no preconditions.
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}
