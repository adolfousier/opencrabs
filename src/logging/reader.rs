//! Reading the log files `logger.rs` writes (#1528).
//!
//! Lives beside the writer because the two share one format: a change to how
//! an entry is emitted has to be a change to how it is parsed, and splitting
//! them across modules is how those drift apart.
//!
//! Two things shape this module, and both come from what is actually on disk
//! rather than from what the format looks like:
//!
//! * **Size.** A day's file here reached 106 MB and 331k lines. Anything that
//!   reads a file to display it would stall the UI and can exhaust memory on a
//!   small box, so [`tail_entries`] seeks from the end and reads a bounded
//!   window.
//! * **Continuation lines.** The format is one entry per line until a logged
//!   message contains a newline, and then it is not. Those spill onto
//!   following lines with no timestamp and no level.

use std::cmp::Ordering;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// How much of the file's tail to read. Chosen so a full screen of entries is
/// always available even when every line is a long JSON payload, while staying
/// small enough to parse instantly on a file hundreds of times this size.
pub const TAIL_BYTES: u64 = 512 * 1024;

/// Severity, ordered so a filter can ask for "this level and worse".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    /// Parse the level field. Returns `None` for anything else, which is the
    /// signal that this line is a continuation rather than a new entry.
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "ERROR" => Some(LogLevel::Error),
            "WARN" => Some(LogLevel::Warn),
            "INFO" => Some(LogLevel::Info),
            "DEBUG" => Some(LogLevel::Debug),
            "TRACE" => Some(LogLevel::Trace),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LogLevel::Error => "ERROR",
            LogLevel::Warn => "WARN",
            LogLevel::Info => "INFO",
            LogLevel::Debug => "DEBUG",
            LogLevel::Trace => "TRACE",
        }
    }

    /// Does this entry pass a "minimum severity" filter?
    ///
    /// `Error` is the most severe and sorts first, so "at least WARN" means
    /// `self <= Warn`. Written out because the comparison reads backwards
    /// from the intent.
    pub fn at_least(self, minimum: LogLevel) -> bool {
        matches!(self.cmp(&minimum), Ordering::Less | Ordering::Equal)
    }

    /// Cycle order for the viewer's level key, most to least severe.
    pub fn all() -> [LogLevel; 5] {
        [
            LogLevel::Error,
            LogLevel::Warn,
            LogLevel::Info,
            LogLevel::Debug,
            LogLevel::Trace,
        ]
    }
}

/// One log entry, which may span several physical lines.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: LogLevel,
    /// Module path the event came from, e.g. `opencrabs::channels::whatsapp`.
    pub target: String,
    /// First line of the message.
    pub message: String,
    /// Lines that followed with no timestamp of their own: JSON payloads,
    /// stack traces, captured command output.
    pub continuation: Vec<String>,
}

impl LogEntry {
    /// Every line this entry renders as, header first.
    pub fn lines(&self) -> usize {
        1 + self.continuation.len()
    }

    /// Does this entry match a substring filter?
    ///
    /// Searches the target, the message, AND the continuation lines. Body text
    /// is the point: the reason to search a log is usually a string inside a
    /// payload or a stack trace, not one in the first line.
    pub fn matches(&self, needle: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        let needle = needle.to_lowercase();
        self.target.to_lowercase().contains(&needle)
            || self.message.to_lowercase().contains(&needle)
            || self
                .continuation
                .iter()
                .any(|l| l.to_lowercase().contains(&needle))
    }
}

/// Split one log line into `(timestamp, level, rest)`, or `None` when it does
/// not begin an entry.
///
/// The shape is `<rfc3339> <LEVEL> ThreadId(N) <target>: <file:line>: <msg>`.
/// A line qualifies only if the second whitespace field is a level AND the
/// first looks like a timestamp: a continuation line whose text happens to
/// start with the word `INFO` must not be mistaken for a new entry.
fn split_header(line: &str) -> Option<(&str, LogLevel, &str)> {
    let (timestamp, rest) = line.split_once(' ')?;
    if !looks_like_timestamp(timestamp) {
        return None;
    }
    let (level_token, rest) = rest.split_once(' ')?;
    let level = LogLevel::parse(level_token)?;
    Some((timestamp, level, rest))
}

/// Cheap shape check for an RFC3339 stamp: `2026-09-13T01:00:00...`.
///
/// Deliberately not a full date parse. This runs per line over a large window,
/// and the job is only to tell a header apart from body text.
fn looks_like_timestamp(token: &str) -> bool {
    let b = token.as_bytes();
    b.len() >= 20
        && b[0..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
        && b[10] == b'T'
}

/// Pull the target out of the part after `ThreadId(N) `.
///
/// Returns `(target, message)`. The target is the segment up to the first
/// `": "`, and the `file.rs:line:` prefix that follows is dropped: it repeats
/// the target with less context and costs a third of the row's width.
fn split_target(rest: &str) -> (String, String) {
    // Skip the thread id, which is noise in a single-process viewer.
    let rest = match rest.split_once(' ') {
        Some((first, tail)) if first.starts_with("ThreadId(") => tail,
        _ => rest,
    };
    let Some((target, after)) = rest.split_once(": ") else {
        return (String::new(), rest.trim().to_string());
    };
    // Drop the `src/path/file.rs:123: ` prefix when present.
    let message = match after.split_once(": ") {
        Some((loc, msg)) if loc.contains(".rs:") => msg,
        _ => after,
    };
    (target.to_string(), message.trim().to_string())
}

/// Parse log text into entries.
///
/// A line with no parseable header belongs to the entry above it. That is the
/// whole reason this is not a `lines().map()`: a naive parser gives those
/// lines a made-up level, and a level filter then drops them. The dropped
/// content is exactly what a reader opened the log for, and worse, the entry's
/// first line survives, so the filter looks like it worked.
///
/// Continuation lines arriving before any header are discarded: they are the
/// tail of an entry that began before the window started, and rendering half a
/// payload under no header would be a mystery rather than information.
pub fn parse(text: &str) -> Vec<LogEntry> {
    let mut entries: Vec<LogEntry> = Vec::new();
    for line in text.lines() {
        match split_header(line) {
            Some((timestamp, level, rest)) => {
                let (target, message) = split_target(rest);
                entries.push(LogEntry {
                    timestamp: timestamp.to_string(),
                    level,
                    target,
                    message,
                    continuation: Vec::new(),
                });
            }
            None => {
                if let Some(last) = entries.last_mut() {
                    last.continuation.push(line.to_string());
                }
            }
        }
    }
    entries
}

/// Read the last [`TAIL_BYTES`] of a log file and parse them.
///
/// The first line of the window is almost always cut mid-entry, so it is
/// dropped unless the window covers the whole file. Keeping it would show a
/// fragment as though it were a complete entry.
pub fn tail_entries(path: &Path, bytes: u64) -> std::io::Result<Vec<LogEntry>> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let from_start = len <= bytes;
    let start = len.saturating_sub(bytes);
    file.seek(SeekFrom::Start(start))?;

    let mut buf = Vec::with_capacity(bytes.min(len) as usize);
    file.take(bytes).read_to_end(&mut buf)?;
    // Logs are UTF-8, but a window boundary can land mid-codepoint, so this
    // must not be a `from_utf8` that fails the whole read on one split char.
    let text = String::from_utf8_lossy(&buf);
    let text = if from_start {
        text.as_ref()
    } else {
        match text.find('\n') {
            Some(i) => &text[i + 1..],
            None => "",
        }
    };
    Ok(parse(text))
}

/// Log files in the profile's log directory, oldest first.
///
/// Sorted by name, which is chronological because the writer names files
/// `opencrabs.YYYY-MM-DD`. Files that do not match the prefix are ignored, so
/// an unrelated file in the directory cannot enter the viewer's day rotation.
pub fn available_logs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("opencrabs."))
        })
        .collect();
    files.sort();
    files
}

/// The profile's log directory, matching where `logger.rs` writes.
pub fn log_dir() -> PathBuf {
    crate::config::opencrabs_home().join("logs")
}
