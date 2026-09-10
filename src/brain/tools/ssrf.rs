//! Shared SSRF guard for every server-side fetch tool (OC-04).
//!
//! `web_scrape`, `http_request`, dynamic HTTP tools, and `browser_navigate` all
//! fetch a caller-influenced URL from the host's network, which is a classic
//! SSRF primitive: point one at `http://169.254.169.254/` (cloud metadata),
//! or an RFC1918 admin panel and it reaches
//! inside. This is the single guard they share so they cannot disagree.
//!
//! Two layers. `validate_url` is a synchronous structural + literal-IP check,
//! usable in a redirect-policy closure. `validate_url_resolved` adds a DNS
//! resolution pass so a public-looking hostname that resolves to an internal
//! address (`metadata.google.internal`, `169.254.169.254.nip.io`) is refused
//! before the request goes out.
//!
//! Host classification goes through [`url::Host`] rather than `host_str()`,
//! which keeps the brackets on IPv6 literals and would let `[::1]` slip past.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use url::{Host, Url};

/// Hostnames that name a cloud metadata endpoint by name rather than IP.
const METADATA_HOSTS: &[&str] = &["metadata.google.internal", "metadata", "metadata.goog"];

/// The reason `ip` must not be fetched, or None when it is a public address.
///
/// IPv4-mapped IPv6 (`::ffff:a.b.c.d`) is unmapped and re-checked as IPv4, so a
/// mapped metadata or loopback address cannot dodge the v4 rules.
pub(crate) fn forbidden_ip(ip: IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(v4) => forbidden_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return forbidden_v4(v4);
            }
            forbidden_v6(v6)
        }
    }
}

fn forbidden_v4(v4: Ipv4Addr) -> Option<&'static str> {
    // Loopback (127.0.0.0/8) is deliberately allowed (maintainer decision):
    // a local-first agent legitimately reaches the operator's own machine, and
    // the guest-driver path that made localhost debug ports an exfil target is
    // closed by the approval owner-gating (OC-01) and deny-by-default ingest
    // (OC-02). The SSRF teeth here are reaching OTHER hosts' internals.
    if v4.is_private() {
        return Some("private (RFC1918)");
    }
    if v4.is_link_local() {
        // 169.254.0.0/16, which includes the 169.254.169.254 metadata endpoint.
        return Some("link-local (incl. cloud metadata)");
    }
    if v4.is_broadcast() {
        return Some("broadcast");
    }
    if v4.is_multicast() {
        return Some("multicast");
    }
    if v4.is_unspecified() {
        return Some("unspecified (0.0.0.0)");
    }
    // CGNAT 100.64.0.0/10 — routable internally on many networks.
    let o = v4.octets();
    if o[0] == 100 && (o[1] & 0xc0) == 0x40 {
        return Some("carrier-grade NAT (100.64.0.0/10)");
    }
    None
}

fn forbidden_v6(v6: Ipv6Addr) -> Option<&'static str> {
    // ::1 loopback allowed, as for IPv4 above.
    if v6.is_multicast() {
        return Some("multicast");
    }
    if v6.is_unspecified() {
        return Some("unspecified (::)");
    }
    let seg = v6.segments();
    if (seg[0] & 0xfe00) == 0xfc00 {
        return Some("unique-local (fc00::/7)");
    }
    if (seg[0] & 0xffc0) == 0xfe80 {
        return Some("link-local (fe80::/10)");
    }
    None
}

/// Structural + literal-IP validation. Returns the parsed URL, or a reason.
/// Does NOT resolve DNS — a bare hostname passes here and is caught by
/// [`validate_url_resolved`]. Safe to call from a synchronous redirect closure.
pub(crate) fn validate_url(url: &str) -> Result<Url, String> {
    let parsed = Url::parse(url).map_err(|e| format!("invalid URL: {e}"))?;

    match parsed.scheme() {
        "http" | "https" => {}
        "file" => return Err("file:// URLs are not allowed".to_string()),
        other => return Err(format!("unsupported scheme: {other}")),
    }

    match parsed.host() {
        None => return Err("URL has no host".to_string()),
        Some(Host::Domain(host)) => {
            let lower = host.to_ascii_lowercase();
            // localhost is allowed (resolves to loopback), same rationale as the
            // loopback IP ranges. Metadata hosts stay blocked.
            if METADATA_HOSTS.contains(&lower.as_str()) {
                return Err("requests to the cloud metadata host are not allowed".to_string());
            }
        }
        Some(Host::Ipv4(v4)) => {
            if let Some(reason) = forbidden_v4(v4) {
                return Err(format!("requests to {reason} addresses are not allowed"));
            }
        }
        Some(Host::Ipv6(v6)) => {
            if let Some(reason) = forbidden_ip(IpAddr::V6(v6)) {
                return Err(format!("requests to {reason} addresses are not allowed"));
            }
        }
    }

    Ok(parsed)
}

/// [`validate_url`] plus a DNS resolution pass: a hostname that resolves to any
/// forbidden address is refused. Use this on the initial fetch; the sync
/// [`validate_url`] guards redirect hops.
pub(crate) async fn validate_url_resolved(url: &str) -> Result<Url, String> {
    let parsed = validate_url(url)?;

    if let Some(Host::Domain(host)) = parsed.host() {
        let host = host.to_string();
        let port = parsed.port_or_known_default().unwrap_or(80);
        let addrs = tokio::net::lookup_host((host.as_str(), port))
            .await
            .map_err(|e| format!("could not resolve host {host}: {e}"))?;
        for addr in addrs {
            if let Some(reason) = forbidden_ip(addr.ip()) {
                return Err(format!(
                    "host {host} resolves to a {reason} address, refusing (SSRF guard)"
                ));
            }
        }
    }

    Ok(parsed)
}

/// A reqwest redirect policy that re-validates every hop with [`validate_url`],
/// capped at `max` hops. A public URL that 302s to an internal one is stopped.
pub(crate) fn redirect_policy(max: usize) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= max {
            return attempt.error(format!("too many redirects (>{max})"));
        }
        match validate_url(attempt.url().as_str()) {
            Ok(_) => attempt.follow(),
            Err(reason) => attempt.error(format!("redirect blocked by SSRF guard: {reason}")),
        }
    })
}
