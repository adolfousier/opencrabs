//! Canonicalization and key derivation for the L1 decision-reuse ring (#1648).
//!
//! Pure functions only — no DB, no model, no config — so the cache's
//! identity rules stand alone for unit tests and are re-derived identically
//! by every caller. Two inputs are "the same decision" when they are equal
//! under [`canonical_json`] and share the
//! (tier, policy_version, normalizer_version) triple ([`decision_key`]).
//! The masking rule set is versioned by [`NORMALIZER_VERSION`]: changing a
//! rule bumps it, which misses every old key — stale normalization can never
//! silently answer for the new semantics.

use regex::Regex;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

/// Version of the masking rule set below. Bump on any change to the regexes
/// or the canonicalization walk: old rows land under a different key, go
/// cold, and are swept by TTL — never reused under changed semantics.
pub const NORMALIZER_VERSION: &str = "v1";

/// Domain separation: an unrelated sha256 user cannot collide with a
/// decision key by coincidence.
const KEY_DOMAIN: &[u8] = b"oc-decision-cache-v1";

/// Field separator (ASCII unit separator), so `("ab", "c")` cannot forge the
/// key of `("a", "bc")`.
const FIELD_SEP: u8 = 0x1f;

/// Canonical JSON text for `value`: object keys sorted recursively, string
/// leaves masked per [`mask_volatile`], array order preserved (arrays are
/// semantically ordered in decision inputs).
pub fn canonical_json(value: &Value) -> String {
    serde_json::to_string(&canonicalize(value)).unwrap_or_else(|e| {
        // Only pathological map keys can make this fail. Surface it, and
        // produce a constant: the key it derives matches no real decision,
        // so the worst case is a miss (a live model call), never a wrong
        // reuse, and never a panic inside a hot decision path.
        tracing::error!("decision normalize: canonical serialization failed: {e}");
        String::new()
    })
}

/// sha256 hex over the domain tag and the four identity parts, separated by
/// [`FIELD_SEP`].
pub fn decision_key(
    tier_id: &str,
    policy_version: &str,
    normalizer_version: &str,
    canonical: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(KEY_DOMAIN);
    for part in [tier_id, policy_version, normalizer_version, canonical] {
        hasher.update([FIELD_SEP]);
        hasher.update(part.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map
                .iter()
                .map(|(k, v)| (k.clone(), canonicalize(v)))
                .collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut out = Map::new();
            for (k, v) in entries {
                out.insert(k, v);
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize).collect()),
        Value::String(s) => Value::String(mask_volatile(s)),
        other => other.clone(),
    }
}

/// Mask volatile substrings out of one string leaf: ISO-ish datetimes, bare
/// dates, UUIDs, IPv4 (optional port), and well-known home/tmp roots.
/// Numbers are NOT masked — in classification inputs a number is usually
/// the signal, not noise. Order matters: datetimes go first so the bare
/// date rule cannot eat their date half.
fn mask_volatile(s: &str) -> String {
    static DATETIME: OnceLock<Regex> = OnceLock::new();
    static DATE: OnceLock<Regex> = OnceLock::new();
    static UUID: OnceLock<Regex> = OnceLock::new();
    static IPV4: OnceLock<Regex> = OnceLock::new();
    static PATH: OnceLock<Regex> = OnceLock::new();

    let datetime = DATETIME.get_or_init(|| {
        Regex::new(r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}(:\d{2})?(\.\d+)?(Z|[+-]\d{2}:?\d{2})?")
            .expect("valid datetime pattern")
    });
    let date = DATE.get_or_init(|| Regex::new(r"\d{4}-\d{2}-\d{2}").expect("valid date pattern"));
    let uuid = UUID.get_or_init(|| {
        Regex::new(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")
            .expect("valid uuid pattern")
    });
    let ipv4 = IPV4.get_or_init(|| {
        Regex::new(r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}(:\d{1,5})?\b")
            .expect("valid ipv4 pattern")
    });
    let path = PATH.get_or_init(|| {
        Regex::new(r#"(?:/Users|/home|/root|/private/tmp|/tmp|/var/folders)/[^\s"']+"#)
            .expect("valid path pattern")
    });

    let s = datetime.replace_all(s, "<ts>");
    let s = date.replace_all(&s, "<date>");
    let s = uuid.replace_all(&s, "<uuid>");
    let s = ipv4.replace_all(&s, "<ip>");
    path.replace_all(&s, "<path>").into_owned()
}
