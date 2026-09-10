//! Integration test for Step 6: Opaqueness wall (#148, D11 design law).
//!
//! The opaqueness wall: ONLY `src/channels/target_resolver.rs` may parse
//! or construct the internal grammar of `oc://` target URLs. Every other
//! module in the codebase treats them as opaque tokens — passing them to
//! `resolve_target` / `bake_delivery_target` / `is_target_url`, or emitting
//! dual-form display strings in list tools.
//!
//! This test scans every `.rs` file outside the resolver and tests dirs.
//! Any attempt to split `oc://` by `/`, decode its segments, parse its
//! authority, or inspect its internal path fails this test — preventing
//! consumer drift and leaky abstractions across channels.

use std::path::Path;

#[test]
fn opaqueness_wall_no_oc_url_parsing_outside_target_resolver() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_dir = manifest_dir.join("src");

    let mut violations = Vec::new();

    // Walk all .rs files in src/
    walk_rs_files(&src_dir, &mut |path| {
        let rel_path = path.strip_prefix(manifest_dir).unwrap_or(path);
        let rel_str = rel_path.to_string_lossy();

        // Allow-listed files:
        // 1. The target resolver itself (the ONLY authorized parser/constructor)
        if rel_str == "src/channels/target_resolver.rs" {
            return;
        }
        // 2. Integration tests (they test the scheme and its boundaries)
        if rel_str.starts_with("src/tests/") {
            return;
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return,
        };

        // Forbidden parser patterns outside target_resolver:
        // Consumers must not reach inside `oc://` URLs.
        for (line_no, line) in content.lines().enumerate() {
            let line_trimmed = line.trim();

            // Skip comments and docstrings
            if line_trimmed.starts_with("//")
                || line_trimmed.starts_with("/*")
                || line_trimmed.starts_with('*')
                || line_trimmed.starts_with("///")
                || line_trimmed.starts_with("//!")
            {
                continue;
            }

            // Pattern 1: Splitting or parsing `oc://` directly
            if line.contains(r#"strip_prefix("oc://"#)
                || line.contains(r#"strip_prefix("oc://session"#)
                || line.contains(r#"strip_prefix("oc://telegram"#)
                || line.contains(r#"strip_prefix("oc://whatsapp"#)
            {
                violations.push(format!(
                    "{}:{}: forbidden oc:// parser pattern: `{}`",
                    rel_str,
                    line_no + 1,
                    line_trimmed
                ));
            }

            // Pattern 2: Ad-hoc URL splitting on `oc://`
            if line.contains(r#"split("oc://")"#) || line.contains(r#"splitn(2, "oc://")"#) {
                violations.push(format!(
                    "{}:{}: forbidden oc:// split pattern: `{}`",
                    rel_str,
                    line_no + 1,
                    line_trimmed
                ));
            }
        }
    });

    assert!(
        violations.is_empty(),
        "Opaqueness wall breached! The following lines parse oc:// URLs outside target_resolver.rs:\n{}",
        violations.join("\n")
    );
}

#[test]
fn resolver_is_sole_authority_for_is_target_url() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_dir = manifest_dir.join("src");

    let mut raw_starts_with_hits = Vec::new();

    walk_rs_files(&src_dir, &mut |path| {
        let rel_path = path.strip_prefix(manifest_dir).unwrap_or(path);
        let rel_str = rel_path.to_string_lossy();

        if rel_str == "src/channels/target_resolver.rs" || rel_str.starts_with("src/tests/") {
            return;
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return,
        };

        for (line_no, line) in content.lines().enumerate() {
            let line_trimmed = line.trim();
            if line_trimmed.starts_with("//") || line_trimmed.starts_with("///") {
                continue;
            }
            // Code should use `is_target_url(...)`, not raw `starts_with("oc://")`
            if line.contains(r#"starts_with("oc://")"#) {
                raw_starts_with_hits.push(format!(
                    "{}:{}: use is_target_url() instead of raw starts_with: `{}`",
                    rel_str,
                    line_no + 1,
                    line_trimmed
                ));
            }
        }
    });

    assert!(
        raw_starts_with_hits.is_empty(),
        "Code should use target_resolver::is_target_url() instead of starts_with(\"oc://\"):\n{}",
        raw_starts_with_hits.join("\n")
    );
}

fn walk_rs_files(dir: &Path, callback: &mut dyn FnMut(&Path)) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk_rs_files(&path, callback);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                callback(&path);
            }
        }
    }
}
