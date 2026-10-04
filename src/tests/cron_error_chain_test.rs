//! #1894: scheduler error logs must print the whole anyhow cause chain.
//!
//! `src/cron/scheduler.rs` logged every failure with plain `Display` (`{e}`),
//! which for `anyhow` prints only the outermost `.context(...)` wrapper and
//! drops the chain underneath it. On a production instance that turned 5,100
//! consecutive failed ticks into 5,100 copies of "Failed to list enabled cron
//! jobs" with no way to tell `SQLITE_BUSY` from a `from_row` mismatch from a
//! disk failure (the outage itself is #1893).
//!
//! The discipline these scans pin:
//!
//! 1. no bare `{e}` survives inside a `tracing::` macro call in this file: log
//!    output carries the alternate flag (`{e:#}`) so the chain reaches the log;
//! 2. the only bare `{e}` sites left are the two `format!` strings delivered
//!    into user chats, where a multi-context chain is noise rather than signal,
//!    and they stay exactly two;
//! 3. the load-bearing tick line named in the issue stays converted.
//!
//! `{e:#}` is byte-identical to `{e}` for `reqwest`/`io` errors, whose
//! `Display` impls ignore the alternate flag, so rule 1 costs nothing at those
//! sites. And `{e}` cannot occur as a substring of `{e:#}` (the flag sits
//! before the closing brace), so scanning for the literal counts bare sites
//! only.

const SCHED: &str = include_str!("../cron/scheduler.rs");

/// Macros whose output is an operator log line: the chain belongs there.
const LOG_MACROS: [&str; 4] = [
    "tracing::error!",
    "tracing::warn!",
    "tracing::info!",
    "tracing::debug!",
];

/// Macros whose output reaches a human chat: the short wrapper belongs there.
const CHAT_MACROS: [&str; 3] = ["format!", "println!", "write!"];

struct Span {
    name: &'static str,
    start: usize,
    /// Index just past the matching close paren.
    end: usize,
}

/// Locate the close paren of the call whose open paren sits at `open`.
/// Skips string literals (with escapes) and line comments so an unbalanced
/// paren inside a message string does not end the call early.
fn close_paren(src: &str, open: usize) -> Option<usize> {
    let b = src.as_bytes();
    let mut depth = 0usize;
    let mut i = open;
    let mut in_string = false;
    let mut escaped = false;
    while i < b.len() {
        if in_string {
            if escaped {
                escaped = false;
            } else if b[i] == b'\\' {
                escaped = true;
            } else if b[i] == b'"' {
                in_string = false;
            }
        } else if b[i] == b'"' {
            in_string = true;
        } else if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        } else if b[i] == b'(' {
            depth += 1;
        } else if b[i] == b')' {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }
        i += 1;
    }
    None
}

fn macro_spans(src: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    for name in LOG_MACROS.iter().chain(CHAT_MACROS.iter()) {
        let mut from = 0usize;
        while let Some(rel) = src[from..].find(name) {
            let start = from + rel;
            let b = src.as_bytes();
            let mut j = start + name.len();
            while j < b.len() && (b[j] as char).is_whitespace() {
                j += 1;
            }
            if j < b.len()
                && b[j] == b'('
                && let Some(end) = close_paren(src, j)
            {
                spans.push(Span { name, start, end });
            }
            from = start + name.len();
        }
    }
    spans
}

/// Innermost macro call enclosing `pos`, if any.
fn enclosing(spans: &[Span], pos: usize) -> Option<&Span> {
    spans
        .iter()
        .filter(|s| s.start <= pos && pos < s.end)
        .min_by_key(|s| s.end - s.start)
}

/// Byte offsets of every bare `{e}` in `src`.
fn bare_error_args(src: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = src[from..].find("{e}") {
        out.push(from + rel);
        from += rel + 3;
    }
    out
}

fn line_of(src: &str, pos: usize) -> usize {
    src[..pos].matches('\n').count() + 1
}

fn lines_of(src: &str, positions: &[usize]) -> Vec<usize> {
    positions.iter().map(|&p| line_of(src, p)).collect()
}

#[test]
fn scheduler_log_macros_never_drop_the_cause_chain() {
    let spans = macro_spans(SCHED);
    let offenders: Vec<usize> = bare_error_args(SCHED)
        .into_iter()
        .filter(|&pos| enclosing(&spans, pos).map(|s| LOG_MACROS.contains(&s.name)) == Some(true))
        .map(|pos| line_of(SCHED, pos))
        .collect();
    assert!(
        offenders.is_empty(),
        "scheduler log lines print Display-only, the anyhow chain is dropped at lines: {offners:?} \
         (fix: use {{e:#}} so the cause chain survives to the log)",
        offners = offenders,
    );
}

#[test]
fn the_only_bare_error_strings_are_the_two_chat_deliveries() {
    let spans = macro_spans(SCHED);
    let bare = bare_error_args(SCHED);
    assert_eq!(
        bare.len(),
        2,
        "the scheduler must hold exactly two bare error strings, both delivered to chats; \
         found {} at lines {:?}",
        bare.len(),
        lines_of(SCHED, &bare),
    );
    let in_chat: Vec<usize> = bare
        .iter()
        .copied()
        .filter(|&pos| enclosing(&spans, pos).map(|s| CHAT_MACROS.contains(&s.name)) == Some(true))
        .collect();
    assert_eq!(
        in_chat.len(),
        2,
        "every bare error string must sit in a chat-facing macro; a log macro dropping the \
         chain is the #1894 regression",
    );
    assert!(
        SCHED.contains("let error_msg = format!(\"{e}\");"),
        "the delivered error string from the agent-error arm must stay Display-only: a cause \
         chain in a user chat is noise"
    );
    assert!(
        SCHED.contains("&format!(\"Cron job '{}' failed: {e}\", job.name)"),
        "the failure delivery message must stay Display-only"
    );
}

#[test]
fn the_tick_error_log_carries_the_chain() {
    // The line the production outage hinged on: one line per minute for 85
    // hours, every copy of it useless without the cause chain.
    assert!(
        SCHED.contains("tracing::error!(\"Cron scheduler tick error: {e:#}\");"),
        "the scheduler tick log must use the alternate Display chain (#1894)"
    );
    assert!(
        !SCHED.contains("Cron scheduler tick error: {e}"),
        "the bare tick log is the regression this issue is about"
    );
    // The startup backfill failure, same wrapper-only class.
    assert!(
        SCHED.contains("Failed to backfill missing next_run_at on startup: {e:#}"),
        "the startup backfill log must carry the chain too"
    );
}
