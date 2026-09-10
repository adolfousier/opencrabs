//! SSRF guard tests for `web_scrape`. Prove the fetcher refuses every
//! non-public target (localhost, private/loopback/link-local IPs, cloud
//! metadata, non-http schemes) and accepts ordinary public URLs.

use crate::brain::tools::ssrf::{forbidden_ip, validate_url};

#[test]
fn accepts_public_http_and_https() {
    assert!(validate_url("https://example.com/article").is_ok());
    assert!(validate_url("http://example.com").is_ok());
    // 8.8.8.8 is a public IP literal — allowed.
    assert!(validate_url("https://8.8.8.8/").is_ok());
}

#[test]
fn returns_parsed_url_for_relative_base() {
    let parsed = validate_url("https://example.com/blog/post").unwrap();
    // The returned Url is usable as a base for resolving relative links.
    let joined = parsed.join("/img/logo.png").unwrap();
    assert_eq!(joined.as_str(), "https://example.com/img/logo.png");
}

#[test]
fn rejects_non_web_schemes() {
    assert!(validate_url("file:///etc/passwd").is_err());
    assert!(validate_url("ftp://example.com/x").is_err());
    assert!(validate_url("gopher://example.com").is_err());
}

#[test]
fn allows_loopback_for_local_dev() {
    // Loopback is deliberately allowed (maintainer decision): a local-first
    // agent reaches the operator's own machine, and the guest-driver path is
    // closed by OC-01/OC-02.
    assert!(validate_url("http://localhost/").is_ok());
    assert!(validate_url("http://127.0.0.1:3000/").is_ok());
    assert!(validate_url("http://[::1]:8080/").is_ok());
}

#[test]
fn rejects_cloud_metadata_endpoint() {
    assert!(validate_url("http://169.254.169.254/latest/meta-data/").is_err());
    // Any link-local 169.254.0.0/16 host is refused.
    assert!(validate_url("http://169.254.1.1/").is_err());
}

#[test]
fn rejects_private_ipv4_ranges() {
    assert!(validate_url("http://10.0.0.5/").is_err());
    assert!(validate_url("http://192.168.1.1/").is_err());
    assert!(validate_url("http://172.16.0.1/").is_err());
}

#[test]
fn rejects_internal_ipv6() {
    // Unique-local address (fc00::/7).
    assert!(validate_url("http://[fd00::1]/").is_err());
    // Link-local.
    assert!(validate_url("http://[fe80::1]/").is_err());
}

#[test]
fn rejects_garbage() {
    assert!(validate_url("not a url").is_err());
    assert!(validate_url("https://").is_err());
}

// ── OC-04: bypasses the old guard let through, now closed ────────────────────

#[test]
fn ipv4_mapped_ipv6_is_unmapped_and_refused() {
    // [::ffff:169.254.169.254] slipped past because the v6 arm never unmapped
    // it. Loopback mapped (::ffff:127.0.0.1) is allowed, like plain loopback.
    assert!(validate_url("http://[::ffff:169.254.169.254]/").is_err());
    assert!(validate_url("http://[::ffff:10.0.0.1]/").is_err());
}

#[test]
fn ipv6_link_local_is_refused() {
    assert!(validate_url("http://[fe80::1]/").is_err());
}

#[test]
fn ipv4_unspecified_and_cgnat_are_refused() {
    assert!(validate_url("http://0.0.0.0/").is_err());
    assert!(validate_url("http://100.64.1.1/").is_err()); // CGNAT
    assert!(validate_url("http://100.127.255.255/").is_err()); // CGNAT upper edge
}

#[test]
fn a_public_100_dot_address_outside_cgnat_is_allowed() {
    // 100.0.0.1 and 100.128.0.1 are public — the /10 mask must not over-block.
    assert!(validate_url("http://100.0.0.1/").is_ok());
    assert!(validate_url("http://100.128.0.1/").is_ok());
}

#[test]
fn the_metadata_hostname_is_refused_by_name() {
    // Resolves internally on GCP; blocked by name so a literal-IP check is not
    // the only line of defense.
    assert!(validate_url("http://metadata.google.internal/").is_err());
}

#[test]
fn the_ip_classifier_covers_every_forbidden_family() {
    use std::net::IpAddr;
    for ip in [
        "10.0.0.1",
        "192.168.1.1",
        "169.254.169.254",
        "0.0.0.0",
        "100.64.0.1",
        "fe80::1",
        "fc00::1",
        "::ffff:169.254.169.254",
    ] {
        let addr: IpAddr = ip.parse().unwrap();
        assert!(
            forbidden_ip(addr).is_some(),
            "{ip} must be classified as forbidden"
        );
    }
    // Loopback is allowed alongside public addresses.
    for ip in [
        "8.8.8.8",
        "1.1.1.1",
        "2606:4700:4700::1111",
        "127.0.0.1",
        "::1",
    ] {
        let addr: IpAddr = ip.parse().unwrap();
        assert!(forbidden_ip(addr).is_none(), "{ip} is public and must pass");
    }
}
