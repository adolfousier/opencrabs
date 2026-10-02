//! #1854: the cooldown guard rule is a mechanism, not a doc comment.
//!
//! `rate_limit::GLOBAL_COOLDOWN` and the governor's virtual clock
//! (`CLOCK_OFFSET_MS`) are process-wide. The test harness runs every module in
//! one process on many threads, so a test that arms a deadline or jumps the
//! clock without holding the locks rewrites another test's state mid-assertion.
//! That is exactly how `ack_suppressed_while_global_cooldown_active` and
//! `global_429_suppresses_gossip_acks_without_counting_them` kept flipping
//! red on `main` while passing in isolation: `telegram_long_rate_limit_test.rs`
//! drove the retry ladder, whose `rate_limit::wait_out` calls
//! `record_global_429` AND `test_support::advance` under `#[cfg(test)]`, while
//! holding neither lock.
//!
//! `telegram_cooldown_lock` documented the rule ("take this guard after
//! `registry_guard`") and nothing enforced it. A comment cannot fail a build.
//! This test reads the sources and makes the rule a build failure: any test
//! body that can reach the global cooldown or the shared clock, directly or
//! through the ladder, must hold `telegram_cooldown_lock::guard()` inside its
//! own body.
//!
//! Three things to know before editing this file:
//! - The scan is per test body, not per file. A file-level check passes as soon
//!   as one test in it holds the lock, which is the hole this closes.
//! - Needles are call-shaped (`name(`), so prose in a doc comment that merely
//!   mentions `wait_out` does not count, while a real call does.
//! - The scan cannot know whether a ladder call's closure actually returns
//!   `RetryAfter` (that is data-dependent), so any call into the ladder counts.
//!   Tests that provably never throttle still take the guard: it costs
//!   serialisation, never correctness.

use std::path::Path;

/// Symbols that touch the global cooldown or the shared virtual clock, directly
/// or indirectly. `send_retrying_rate_limit(` and `wait_out(` are here because
/// the ladder arms the cooldown through `rate_limit::wait_out`
/// (`src/channels/telegram/rate_limit.rs:179`) and jumps the clock two lines
/// later (`:196-197`), which is how a test that never names either symbol still
/// corrupts both.
const NEEDLES: &[&str] = &[
    "record_global_429(",
    "reset_global_cooldown(",
    "is_global_cooldown_active(",
    "reaction_ack_permitted(",
    "wait_out(",
    "send_retrying_rate_limit(",
    "ts::reset(",
    "ts::advance(",
    "test_support::reset(",
    "test_support::advance(",
];

/// The guard every needle-bearing test body must hold. Matching the call site
/// rather than an import: `use` proves nothing; holding it across the body's own
/// awaits is what serialises the state.
const GUARD: &str = "telegram_cooldown_lock::guard(";

/// The lock's own module, and this file (whose needle list is string literals).
const EXEMPT: &[&str] = &[
    "telegram_cooldown_lock.rs",
    "telegram_cooldown_discipline_1854_test.rs",
];

/// `(file, fn name, needles matched, holds the guard)`.
type Row = (String, String, String, bool);

/// End index (exclusive) of the block whose opening brace sits at `open`.
///
/// Braces inside comments, strings, raw strings and char literals are content,
/// not structure. `brain_provider_json_repair_test.rs::closes_open_string` hands
/// the repair parser `r#"{"command":"git status"#`: counting that `{` reported an
/// unbalanced body and panicked the scan on a file that has no contact with the
/// cooldown at all. A scanner that dies on its first raw string governs nothing.
fn find_body_end(src: &str, open: usize, file: &str, name: &str) -> usize {
    let bytes = src.as_bytes();
    let mut i = open;
    let mut depth = 0usize;
    while i < bytes.len() {
        if let Some(next) = skip_literal(bytes, i) {
            i = next;
            continue;
        }
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    panic!(
        "{file}: unbalanced braces in test fn {name}, the discipline scan cannot \
         trust itself here"
    )
}

/// If `bytes[i]` opens a comment or a literal, return the index just past it;
/// `None` means the byte is ordinary code. Each token is consumed whole, so the
/// caller never looks inside one and cannot read `//` in a URL string as a
/// comment, or a `"` in a comment as a string.
fn skip_literal(bytes: &[u8], i: usize) -> Option<usize> {
    let rest = &bytes[i..];
    if rest.starts_with(b"//") {
        let mut j = i + 2;
        while j < bytes.len() && bytes[j] != b'\n' {
            j += 1;
        }
        return Some(j);
    }
    if rest.starts_with(b"/*") {
        let mut j = i + 2;
        let mut nesting = 1usize;
        while j < bytes.len() && nesting > 0 {
            if bytes[j..].starts_with(b"/*") {
                nesting += 1;
                j += 2;
            } else if bytes[j..].starts_with(b"*/") {
                nesting -= 1;
                j += 2;
            } else {
                j += 1;
            }
        }
        return Some(j);
    }
    if bytes[i] == b'\'' {
        return char_end(bytes, i);
    }
    // `b".."` and `br#".."#` only where `b` is really a prefix, and `r` only
    // where `#*"` follows, so identifiers spelled with these letters stay code.
    let mut head = i;
    if bytes[head] == b'b'
        && head + 1 < bytes.len()
        && (bytes[head + 1] == b'"' || bytes[head + 1] == b'r')
    {
        head += 1;
    }
    if bytes[head] == b'r' {
        let mut hashes = 0usize;
        let mut k = head + 1;
        while k < bytes.len() && bytes[k] == b'#' {
            hashes += 1;
            k += 1;
        }
        return if bytes.get(k) == Some(&b'"') {
            Some(raw_string_end(bytes, k + 1, hashes))
        } else {
            None
        };
    }
    if bytes[head] == b'"' {
        return Some(string_end(bytes, head + 1));
    }
    None
}

/// Closing `"` of an escaped string literal, exclusive.
fn string_end(bytes: &[u8], mut j: usize) -> usize {
    while j < bytes.len() {
        match bytes[j] {
            b'\\' => j += 2,
            b'"' => return j + 1,
            _ => j += 1,
        }
    }
    bytes.len()
}

/// Closing `"###` of a raw string carrying `hashes` hashes, exclusive. A `"`
/// not followed by the full delimiter is content.
fn raw_string_end(bytes: &[u8], mut j: usize, hashes: usize) -> usize {
    while j < bytes.len() {
        if bytes[j] == b'"' {
            let mut k = j + 1;
            let mut seen = 0usize;
            while k < bytes.len() && bytes[k] == b'#' {
                seen += 1;
                k += 1;
            }
            if seen == hashes {
                return k;
            }
        }
        j += 1;
    }
    bytes.len()
}

/// `'a'` and `'\n'` are char literals; `&'a str` and `'static` are lifetimes.
/// An unrecognised shape consumes the apostrophe alone, which leaves the scan
/// counting braces after it, exactly as the naive version always did.
fn char_end(bytes: &[u8], i: usize) -> Option<usize> {
    let after = *bytes.get(i + 1)?;
    if after != b'\\' {
        return if bytes.get(i + 2) == Some(&b'\'') {
            Some(i + 3)
        } else {
            Some(i + 1)
        };
    }
    let escape = *bytes.get(i + 2)?;
    let mut j = i + 3;
    if escape == b'x' {
        while j < bytes.len() && (bytes[j].is_ascii_hexdigit() || bytes[j] == b'_') {
            j += 1;
        }
    } else if escape == b'u' {
        if bytes.get(j) != Some(&b'{') {
            return Some(i + 1);
        }
        while j < bytes.len() && bytes[j] != b'}' {
            j += 1;
        }
        j += 1;
    }
    if bytes.get(j) == Some(&b'\'') {
        Some(j + 1)
    } else {
        Some(i + 1)
    }
}

/// Every `#[test]` / `#[tokio::test(..)]` body in `src`, with its fn name.
///
/// Brace depth is counted from the opening `{`, so a body ends exactly where
/// the fn does. A body that never closes is an integrity problem worth failing
/// on: silently checking half a file is how a guard test goes vacuous.
fn test_bodies(src: &str, file: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut at = 0usize;
    while let Some(rel) = src[at..].find("#[") {
        let start = at + rel;
        // An attribute inside a line or doc comment is an example, not a test.
        let line_start = src[..start].rfind('\n').map(|n| n + 1).unwrap_or(0);
        if src[line_start..start].trim_start().starts_with("//") {
            at = start + 2;
            continue;
        }
        let after = &src[start + 2..];
        let tagged = after
            .strip_prefix("tokio::test")
            .or_else(|| after.strip_prefix("test"));
        let Some(tail) = tagged else {
            at = start + 2;
            continue;
        };
        // `#[test]` / `#[test(...)]` only: `#[test_case]` and friends are not ours.
        if !(tail.starts_with(']') || tail.starts_with('(')) {
            at = start + 2;
            continue;
        }
        let Some(attr_close) = tail.find(']') else {
            at = start + 2;
            continue;
        };
        let sig_area = &tail[attr_close + 1..];
        let Some(fn_rel) = find_fn(sig_area) else {
            at = start + 2;
            continue;
        };
        let after_fn = &sig_area[fn_rel + 3..];
        let name: String = after_fn
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() {
            at = start + 2;
            continue;
        }
        let brace_rel = after_fn
            .find('{')
            .unwrap_or_else(|| panic!("{file}: test fn {name} has no opening brace"));
        let body_start = start + 2 + attr_close + 1 + fn_rel + 3 + brace_rel;
        let end = find_body_end(src, body_start, file, &name);
        found.push((name, src[body_start..end].to_string()));
        at = start + 2;
    }
    found
}

/// Position of a word-boundary `fn ` (so `some_fn(` or `random` do not match).
fn find_fn(hay: &str) -> Option<usize> {
    let mut at = 0usize;
    while let Some(rel) = hay[at..].find("fn ") {
        let pos = at + rel;
        let boundary = pos == 0
            || !hay.as_bytes()[pos - 1].is_ascii_alphanumeric() && hay.as_bytes()[pos - 1] != b'_';
        if boundary {
            return Some(pos);
        }
        at = pos + 1;
    }
    None
}

/// Read `src/tests/*.rs` (top level only: `fixtures/` is data, not tests) and
/// return every test body that can reach global Telegram state.
fn needle_rows() -> (usize, usize, Vec<Row>) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tests");
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{} unreadable: {e}", dir.display()))
        .map(|e| e.expect("unreadable dir entry").path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "rs"))
        .collect();
    entries.sort();
    // Capture the count BEFORE the loop: `for path in entries` consumes the
    // Vec, so asking it for its length after the loop is a move error.
    let files = entries.len();
    let mut rows = Vec::new();
    let mut bodies = 0usize;
    for path in entries {
        let file = path
            .file_name()
            .expect("no file name")
            .to_string_lossy()
            .to_string();
        if EXEMPT.contains(&file.as_str()) {
            continue;
        }
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{} unreadable: {e}", path.display()));
        for (function, body) in test_bodies(&src, &file) {
            bodies += 1;
            let hit: Vec<&str> = NEEDLES
                .iter()
                .copied()
                .filter(|n| body.contains(n))
                .collect();
            if hit.is_empty() {
                continue;
            }
            rows.push((file.clone(), function, hit.join(","), body.contains(GUARD)));
        }
    }
    (files, bodies, rows)
}

#[test]
fn every_test_that_touches_the_global_cooldown_holds_the_guard() {
    let (_, _, rows) = needle_rows();
    let unaccounted: Vec<&Row> = rows.iter().filter(|(_, _, _, holds)| !holds).collect();
    assert!(
        unaccounted.is_empty(),
        "{} test(s) move the shared Telegram clock or reach the process-wide 429 \
         cooldown without holding the cooldown guard. Take \
         `crate::tests::telegram_cooldown_lock::guard().await` AFTER \
         `test_support::registry_guard().await` (that order everywhere, so no \
         lock-order inversion is possible): #1854. The rule itself lives in \
         `src/tests/telegram_cooldown_lock.rs`; this scan is what makes it real.\n\
         Holes:\n{}",
        unaccounted.len(),
        unaccounted
            .iter()
            .map(|(f, n, needles, _)| format!("  {f}::{n}  reaches: {needles}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The scan must keep seeing the population it governs.
///
/// An empty offender list is only reassuring if the scan was not blind, so these
/// floors fail loudly when the parser, the directory, or the witnesses
/// themselves go missing. Without them a broken scanner reads as a clean bill of
/// health, which is worse than no scanner at all.
#[test]
fn the_scan_still_sees_the_tests_it_governs() {
    let (files, bodies, rows) = needle_rows();
    assert!(
        files > 500,
        "only {files} test files scanned from src/tests"
    );
    assert!(
        bodies > 5_000,
        "only {bodies} test bodies scanned: the attribute or \
         brace parsing broke, so the guard rule is unenforced"
    );
    let guarded = rows.iter().filter(|(_, _, _, holds)| *holds).count();
    let total = rows.len();
    assert!(
        total >= 31 && guarded == total,
        "cooldown-touching bodies: {total} found, {guarded} guarded. The floor of 31 \
         is the population #1854 accounted for (20 holes plus the files that \
         already complied); if it dropped, guards were removed or the needles stopped matching."
    );
    for (file, expected) in [
        ("governor_gates_test.rs", 11usize),
        ("telegram_long_rate_limit_test.rs", 3),
        ("telegram_send_retry_test.rs", 6),
        ("telegram_ack_cooldown_gate_test.rs", 3),
    ] {
        let seen = rows.iter().filter(|(f, ..)| f == file).count();
        assert!(
            seen >= expected,
            "{file}: the scan found {seen} cooldown-touching bodies, expected at \
             least {expected}. Either the guards this issue required are gone, or \
             the scan's matching broke; a silent scan is not a guard."
        );
    }
}

/// The scan is pinned against its own failure mode. `brain_provider_json_repair_test.rs`
/// broke the naive counter on a raw string, so this asserts the tricky tokens
/// directly: a body containing an unmatched `{` inside `r#"..."#`, a `}` in a
/// line comment, `'}'` as a char literal, and a `{` inside a string still
/// yields exactly one body, closed at its own brace.
#[test]
fn literals_and_comments_do_not_move_the_brace_census() {
    let src = r##"#[test]
fn tricky() {
let json = r#"{"open":"unclosed"#;
// a line comment with } and {
/* a block comment with } */
let brace = '}';
let text = "{ not a block";
}
"##;
    let bodies = test_bodies(src, "inline.rs");
    assert_eq!(bodies.len(), 1, "the body must close on its own brace");
    assert_eq!(bodies[0].0, "tricky");
    assert!(
        bodies[0].1.ends_with('}'),
        "captured body does not end at the closing brace: {:?}",
        &bodies[0].1[bodies[0].1.len().saturating_sub(24)..]
    );
    assert!(brace_is_balanced(&bodies[0].1));
}

/// True when the body's own braces balance without ever going negative, checked
/// with the same literal-aware walk the scanner uses.
fn brace_is_balanced(body: &str) -> bool {
    let bytes = body.as_bytes();
    let mut i = 0usize;
    let mut depth = 0i64;
    while i < bytes.len() {
        if let Some(next) = skip_literal(bytes, i) {
            i = next;
            continue;
        }
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    depth == 0
}
