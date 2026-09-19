//! Crash-signal handler stub for targets with no implemented fault handler.
//!
//! macOS, Windows and non-x86_64 Linux are built by this crate's release
//! workflow, so the symbol must exist for them to link. The register and
//! `ucontext_t` layout the real handler reads is architecture- and OS-specific,
//! and a wrong read there would report a garbage fault address as fact, so this
//! returns an error and the absence stays visible to the caller instead of the
//! daemon believing it is covered.
//!
//! Lives in its own file rather than a `#[cfg(not(...))]` module inside
//! `crash.rs`, so that `crash.rs` compiles only where its record formatter has
//! a consumer. Compiled everywhere, the formatter is dead code on every target
//! but one.

use std::io::{Error, ErrorKind};

/// Always fails.
///
/// # Errors
///
/// Always returns [`ErrorKind::Unsupported`]: the fault handler is
/// implemented for x86_64 Linux only.
pub fn install_crash_handler() -> std::io::Result<()> {
    Err(Error::new(
        ErrorKind::Unsupported,
        "crash-signal handler is implemented for x86_64 Linux only",
    ))
}
