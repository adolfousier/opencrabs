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
//! SAFETY: Unix-only (stubbed as no-op elsewhere). Only call while the TUI
//! is in alternate screen. Brief fd suppression during a render tick just
//! means one skipped frame.

/// Redirect stdout AND stderr to /dev/null until the guard is dropped.
///
/// Returns `None` if the dup/open calls fail. Callers should treat `None`
/// as "suppression not active" and proceed without retry.
#[cfg(unix)]
pub fn suppress_stdio() -> Option<StdioGuard> {
    use std::os::unix::io::AsRawFd;
    unsafe {
        let stdout_fd = std::io::stdout().as_raw_fd();
        let stderr_fd = std::io::stderr().as_raw_fd();
        let saved_stdout = libc::dup(stdout_fd);
        if saved_stdout < 0 {
            return None;
        }
        let saved_stderr = libc::dup(stderr_fd);
        if saved_stderr < 0 {
            libc::close(saved_stdout);
            return None;
        }
        let devnull = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY);
        if devnull < 0 {
            libc::close(saved_stdout);
            libc::close(saved_stderr);
            return None;
        }
        libc::dup2(devnull, stdout_fd);
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
    saved_stdout: i32,
    saved_stderr: i32,
}

#[cfg(unix)]
impl Drop for StdioGuard {
    fn drop(&mut self) {
        use std::os::unix::io::AsRawFd;
        unsafe {
            let stdout_fd = std::io::stdout().as_raw_fd();
            let stderr_fd = std::io::stderr().as_raw_fd();
            libc::dup2(self.saved_stdout, stdout_fd);
            libc::dup2(self.saved_stderr, stderr_fd);
            libc::close(self.saved_stdout);
            libc::close(self.saved_stderr);
        }
    }
}

#[cfg(not(unix))]
pub fn suppress_stdio() -> Option<StdioGuard> {
    None
}

#[cfg(not(unix))]
pub struct StdioGuard;
