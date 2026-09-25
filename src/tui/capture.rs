//! Byte-capture for the TUI write stream (#1719).
//!
//! Every byte Crossterm writes to the terminal passes through
//! [`CaptureWriter`], which tees it into a fixed in-memory ring buffer.
//! When a render panic is caught, the tail of that ring is dumped into the
//! error log: if the tail contains raw escape junk (CSI/OSC sequences that
//! leaked in through model or tool text), the on-screen corruption came from
//! unsanitized content being executed by the terminal; if the stream looks
//! clean, the corruption came from the render itself. That distinction is
//! exactly what the #1719 garble report could not provide.
//!
//! With `OPENCRABS_TUI_CAPTURE=1`, the full byte stream additionally appends
//! to `~/.opencrabs/logs/tui-capture.bin` for post-mortem of panic-free
//! garble episodes. Capture failures are silently ignored: observability
//! must never break rendering.

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

/// Ring size: 64 KB of terminal bytes is several full frames, enough to see
/// what was written immediately before a caught panic without growing
/// unbounded across a long session.
pub(crate) const RING_CAPACITY: usize = 64 * 1024;

/// Fixed-capacity ring buffer holding the most recent bytes written to the
/// terminal. Oldest bytes are overwritten once the capacity is reached.
pub(crate) struct CaptureRing {
    buf: Vec<u8>,
    /// Next write position (wraps at capacity).
    pos: usize,
    /// Total bytes ever pushed, capped at capacity for snapshot math.
    filled: usize,
}

impl CaptureRing {
    pub(crate) fn with_capacity(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
            pos: 0,
            filled: 0,
        }
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) {
        let cap = self.buf.capacity().max(1);
        for &b in bytes {
            if self.buf.len() < cap {
                self.buf.push(b);
            } else {
                self.buf[self.pos] = b;
            }
            self.pos = (self.pos + 1) % cap;
        }
        self.filled = self.filled.saturating_add(bytes.len()).min(cap);
    }

    /// Snapshot in write order: oldest surviving byte first.
    pub(crate) fn snapshot(&self) -> Vec<u8> {
        let cap = self.buf.capacity();
        if self.buf.len() < cap || self.pos == 0 {
            return self.buf.clone();
        }
        let mut out = Vec::with_capacity(self.buf.len());
        out.extend_from_slice(&self.buf[self.pos..]);
        out.extend_from_slice(&self.buf[..self.pos]);
        out
    }
}

static RING: LazyLock<Mutex<CaptureRing>> =
    LazyLock::new(|| Mutex::new(CaptureRing::with_capacity(RING_CAPACITY)));

/// Record bytes into the global ring. Never panics, never blocks on poison:
/// a poisoned lock just means capture is degraded, which beats crashing the
/// render loop over diagnostics.
pub(crate) fn record(bytes: &[u8]) {
    if let Ok(mut ring) = RING.lock() {
        ring.push(bytes);
    }
}

/// Ordered copy of the surviving ring contents.
pub(crate) fn snapshot() -> Vec<u8> {
    RING.lock().map(|ring| ring.snapshot()).unwrap_or_default()
}

fn capture_file_enabled() -> bool {
    std::env::var("OPENCRABS_TUI_CAPTURE").is_ok_and(|v| !v.is_empty() && v != "0")
}

fn capture_file_path() -> PathBuf {
    let home = std::env::var("HOME")
        .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned());
    PathBuf::from(home)
        .join(".opencrabs")
        .join("logs")
        .join("tui-capture.bin")
}

fn append_capture_file(bytes: &[u8]) {
    let path = capture_file_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(bytes);
    }
}

/// Wraps the terminal's writer so every byte ratatui/crossterm emit is also
/// recorded. Pass-through cost is one memcpy into the ring per write.
pub(crate) struct CaptureWriter<W: io::Write> {
    inner: W,
}

impl<W: io::Write> CaptureWriter<W> {
    pub(crate) fn new(inner: W) -> Self {
        Self { inner }
    }
}

impl<W: io::Write> Write for CaptureWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        record(buf);
        if capture_file_enabled() {
            append_capture_file(buf);
        }
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Render bytes for logs: printable ASCII stays as-is, everything else
/// becomes `\xNN`, so escape-sequence junk is visible instead of being
/// executed by whatever views the log.
pub(crate) fn escape_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        match b {
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            0x20..=0x7e => out.push(b as char),
            _ => out.push_str(&format!("\\x{b:02x}")),
        }
    }
    out
}

/// Dump the tail of the captured stream into the error log. Called from the
/// caught-render-panic path: the tail shows what the terminal was actually
/// fed right before the panic, separating bad-content corruption from
/// bad-layout panics.
pub(crate) fn dump_tail_to_log(context: &str) {
    const TAIL: usize = 2048;
    let snap = snapshot();
    let start = snap.len().saturating_sub(TAIL);
    let tail = &snap[start..];
    tracing::error!(
        "[TUI] byte-stream tail ({} of {} bytes) {}: {}",
        tail.len(),
        snap.len(),
        context,
        escape_bytes(tail)
    );
}
