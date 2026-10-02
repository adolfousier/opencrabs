//! Panic persistence for panics the terminal swallows (#1791).
//!
//! The TUI installs a panic hook (`src/tui/runner.rs`) whose job is to put the
//! terminal back into a usable state before the panic text hits stderr. It
//! already computed everything a post-mortem needs — source location, a
//! force-captured backtrace, and the first `opencrabs::` frame — and then
//! dropped all of it on the floor. A panic outside the render `catch_unwind`
//! unwound out of the event loop, the process exited, and the evidence went
//! with it: an instance that died between 22:12:59 and 22:18:41 left exactly
//! zero trace in the daily log, so "debug the logs to catch that panic" is
//! unanswerable by construction.
//!
//! Two sinks, deliberately:
//!
//! 1. `tracing::error!` so the panic lands in the same daily rolling file as
//!    everything else a maintainer greps. The file writer is synchronous
//!    (see `ResilientFileWriter` in `logger.rs` — no `non_blocking` worker),
//!    so the line is on disk before the hook returns rather than queued to a
//!    thread that never gets scheduled.
//! 2. A direct append to `<log_dir>/panic.log`, which does not depend on the
//!    tracing file layer being enabled. Debug logs are a config toggle; a
//!    death record cannot be allowed to hide behind one, and stderr is
//!    ephemeral — a closed terminal window took the evidence with it.
//!
//! Like `logging::crash` (#352), the interesting half is pure: a record, the
//! renderers that turn it into bytes, and a payload reader that never panics.
//! The only effectful entry point is [`persist`].

use std::any::Any;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Filename of the panic record inside [`crate::logging::logger::log_dir`].
/// Appended to, never rotated: one record per panic, oldest first. A process
/// that panics often enough for this to matter has bigger problems than file
/// size, and unlike the daily logs this one must never be pruned out from
/// under an investigation. That is not a wish: `cleanup_old_logs` only deletes
/// entries `is_log_file()` accepts, and `is_log_file()` requires the
/// `opencrabs` prefix followed by `.`, which `panic.log` never has.
pub(crate) const PANIC_LOG_NAME: &str = "panic.log";

/// Upper bound on the backtrace written per record. `Backtrace::force_capture`
/// is unbounded; a pathological stack (deep recursion is a common way to panic)
/// would otherwise be persisted in full. The head is the useful part — the
/// frames nearest the panic.
pub(crate) const BACKTRACE_CAP: usize = 64 * 1024;

/// Marker appended when [`BACKTRACE_CAP`] truncated the stack.
pub(crate) const TRUNCATION_MARK: &str = "... [backtrace truncated at 65536 bytes]";

/// The facts a panic hook is handed, minus the hook.
///
/// Plain data on purpose: the alternative is a test that has to raise a real
/// panic to check a string layout, and a test that aborts the suite is not a
/// test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanicRecord {
    /// Source file reported by the hook (`None` when the panic site is
    /// unknown, which happens for panics raised from `core` without location).
    pub(crate) file: Option<String>,
    /// Line within [`Self::file`].
    pub(crate) line: Option<u32>,
    /// Column within [`Self::file`].
    pub(crate) column: Option<u32>,
    /// The panic payload as a string.
    pub(crate) message: String,
    /// First `opencrabs::` frame of the captured backtrace, if one was found.
    pub(crate) opencrabs_frame: Option<String>,
}

impl PanicRecord {
    /// `<file>:<line>:<col>`, or `unknown location` when the hook had none.
    pub(crate) fn location(&self) -> String {
        match (self.file.as_deref(), self.line) {
            (Some(file), Some(line)) => match self.column {
                Some(column) => format!("{file}:{line}:{column}"),
                None => format!("{file}:{line}"),
            },
            _ => "unknown location".to_string(),
        }
    }

    /// The one-line shape shared by both sinks. Prefixed with `PANIC` so a
    /// maintainer can grep one word across the daily log and `panic.log` and
    /// find every death record. One record per panic, always from the hook:
    /// it is the only place that sees both the panics the render loop catches
    /// and the ones that unwind out of it, and guessing which one it was at
    /// hook time is not information, it is a coin flip.
    pub(crate) fn header(&self) -> String {
        match self.opencrabs_frame.as_deref() {
            Some(frame) => format!("PANIC {} {frame} :: {}", self.location(), self.message),
            None => format!("PANIC {} :: {}", self.location(), self.message),
        }
    }

    /// The multi-line body appended to `panic.log`: the header, then the raw
    /// backtrace, capped at [`BACKTRACE_CAP`] on a byte boundary that cannot
    /// split a UTF-8 character (a stack frame carrying a non-ASCII symbol name
    /// is enough to turn a naive slice into a panic, inside a panic handler).
    pub(crate) fn report(&self, backtrace: &str) -> String {
        let mut out = String::with_capacity(self.header().len() + 1024);
        out.push_str(&self.header());
        out.push('\n');
        if backtrace.is_empty() {
            out.push_str("backtrace: none captured\n");
            return out;
        }
        if backtrace.len() <= BACKTRACE_CAP {
            out.push_str(backtrace);
        } else {
            let mut end = BACKTRACE_CAP;
            while end > 0 && !backtrace.is_char_boundary(end) {
                end -= 1;
            }
            out.push_str(&backtrace[..end]);
            out.push('\n');
            out.push_str(TRUNCATION_MARK);
        }
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out
    }
}

/// Read a panic payload without ever failing.
///
/// `PanicHookInfo::payload` hands back a `&(dyn Any + Send)` (not `Sync`;
/// the payload only has to cross the panic boundary, not a thread), and
/// `unwrap`ing the downcast is
/// the classic way to panic *inside* the panic handler — which aborts the
/// process and destroys the very evidence this module exists to keep.
/// Anything unexpected comes back as an explicit "not a string" description
/// rather than a `None`.
pub(crate) fn payload_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<char>() {
        s.to_string()
    } else {
        "panic payload was not a string".to_string()
    }
}

/// Where the record goes for a given log directory. Pure; the caller decides
/// what "the log directory" is so this stays testable without touching home.
pub(crate) fn panic_log_path(log_dir: &Path) -> PathBuf {
    log_dir.join(PANIC_LOG_NAME)
}

/// Re-entrancy latch. A panic raised while persisting a panic would otherwise
/// recurse through the hook until the stack gives out.
static PERSISTING: AtomicBool = AtomicBool::new(false);

/// RAII latch holder: entered once, released on drop.
///
/// `pub(crate)` rather than private so the latch itself can be tested
/// deterministically: a race-based test of a re-entrancy guard either cannot
/// reproduce the collision or cannot be trusted to.
pub(crate) struct PersistGuard;

impl PersistGuard {
    pub(crate) fn try_enter() -> Option<Self> {
        // `swap` hands back the previous value: `true` means a record is
        // already in flight and this call is the recursion we exist to stop.
        if PERSISTING.swap(true, Ordering::AcqRel) {
            None
        } else {
            Some(Self)
        }
    }
}

impl Drop for PersistGuard {
    fn drop(&mut self) {
        PERSISTING.store(false, Ordering::Release);
    }
}

/// Write one record: latch, path, then [`persist_into`].
///
/// Returns the path written, or `None` when this call was refused because
/// another record is already being persisted.
pub(crate) fn persist(record: &PanicRecord, backtrace: &str, log_dir: &Path) -> Option<PathBuf> {
    let _guard = PersistGuard::try_enter()?;
    let path = panic_log_path(log_dir);
    persist_into(record, backtrace, &path).then_some(path)
}

/// The effects themselves, latch-free: the greppable line to `tracing` and
/// the full body appended to `path`.
///
/// Split from [`persist`] so the write path is testable without engaging the
/// process-global latch. The latch exists to stop a panic *while panicking*
/// from recursing; two ordinary tests are not that, and a suite that contends
/// on a global for a reason it does not share is just a future flake.
///
/// Best-effort by design: an I/O failure is reported through `tracing` if that
/// sink works at all and never propagated, because the caller is dying and has
/// nowhere to put an error.
///
/// Returns whether the record reached the file.
pub(crate) fn persist_into(record: &PanicRecord, backtrace: &str, path: &Path) -> bool {
    tracing::error!("{}", record.header());
    match write_record(path, &record.report(backtrace)) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("panic record not written to {}: {e}", path.display());
            false
        }
    }
}

/// Create the directory if missing and append the body. Split out so the
/// behavioural test exercises the real write path, including the missing-dir
/// case that a crash hook cannot rely on having been created by anything else.
fn write_record(path: &Path, body: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(body.as_bytes())?;
    file.flush()
}
