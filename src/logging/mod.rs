//! Logging and Debug System

#[cfg(unix)]
pub(crate) mod crash;
pub(crate) mod logger;
pub(crate) mod reader;
pub(crate) mod redact;

#[cfg(unix)]
pub use crash::install_crash_handler;
pub use logger::*;
