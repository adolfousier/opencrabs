//! Temporary fd-level redirection of stdout/stderr to `/dev/null`.
//!
//! Used when invoking libraries that write to stdout/stderr directly
//! (hf-hub's indicatif progress bar, kalosm-common's CPU/GPU println, etc.)
//! while the TUI owns the terminal in raw mode + alternate screen. Without
//! this, the progress bar bleeds into the UI.
//!
//! Usage:
//! ```ignore
//! let _guard = fd_suppress::suppress_stdio();
//! call_library_that_prints_to_stderr();
//! // guard drops here, fds restored
//! ```
//!
//! When the TUI is active (`set_tui_active(true)`), only stderr is suppressed.
//! Redirecting stdout (fd 1) while ratatui is writing escape sequences causes
//! partial sequences → garbled display. `silence_llama_logs()` already silences
//! llama.cpp's stdout via tracing, so stdout suppression is only needed in
//! headless/CLI mode.

use std::sync::atomic::{AtomicBool, Ordering};

/// Flag indicating the TUI event loop owns stdout.
/// When set, `suppress_stdio()` skips stdout redirection to avoid racing
/// with ratatui's terminal writes on fd 1.
static TUI_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Mark the TUI as active/inactive. Call with `true` before entering the
/// event loop and `false` after leaving it.
pub fn set_tui_active(active: bool) {
    TUI_ACTIVE.store(active, Ordering::SeqCst);
}

/// Redirect stdout and/or stderr to /dev/null until the guard is dropped.
///
/// When the TUI is active, only stderr is suppressed (stdout stays untouched
/// so ratatui's escape sequences reach the terminal intact).
///
/// Returns `None` if the dup/open calls fail. Callers should treat `None`
/// as "suppression not active" and proceed without retry.
#[cfg(unix)]
pub fn suppress_stdio() -> Option<StdioGuard> {
    use std::os::unix::io::AsRawFd;

    let tui_active = TUI_ACTIVE.load(Ordering::SeqCst);

    unsafe {
        let stderr_fd = std::io::stderr().as_raw_fd();
        let saved_stderr = libc::dup(stderr_fd);
        if saved_stderr < 0 {
            return None;
        }

        // Only save/redirect stdout when the TUI is NOT active.
        let saved_stdout = if tui_active {
            -1 // sentinel: stdout was not redirected
        } else {
            let stdout_fd = std::io::stdout().as_raw_fd();
            let fd = libc::dup(stdout_fd);
            if fd < 0 {
                libc::close(saved_stderr);
                return None;
            }
            fd
        };

        let devnull = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY);
        if devnull < 0 {
            if saved_stdout >= 0 {
                libc::close(saved_stdout);
            }
            libc::close(saved_stderr);
            return None;
        }

        if saved_stdout >= 0 {
            libc::dup2(devnull, std::io::stdout().as_raw_fd());
        }
        libc::dup2(devnull, stderr_fd);
        libc::close(devnull);

        Some(StdioGuard {
            saved_stdout,
            saved_stderr,
        })
    }
}

#[cfg(unix)]
pub struct StdioGuard {
    saved_stdout: i32, // -1 = stdout was not redirected (TUI active)
    saved_stderr: i32,
}

#[cfg(unix)]
impl Drop for StdioGuard {
    fn drop(&mut self) {
        use std::os::unix::io::AsRawFd;
        unsafe {
            let stderr_fd = std::io::stderr().as_raw_fd();
            libc::dup2(self.saved_stderr, stderr_fd);
            libc::close(self.saved_stderr);

            if self.saved_stdout >= 0 {
                let stdout_fd = std::io::stdout().as_raw_fd();
                libc::dup2(self.saved_stdout, stdout_fd);
                libc::close(self.saved_stdout);
            }
        }
    }
}

#[cfg(not(unix))]
pub fn suppress_stdio() -> Option<StdioGuard> {
    None
}

#[cfg(not(unix))]
pub struct StdioGuard;
