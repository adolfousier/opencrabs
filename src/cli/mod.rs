//! CLI Module
//!
//! Command-line interface for OpenCrabs using Clap v4.

mod args;
mod commands;
pub(crate) mod crash_recovery;
mod cron;
mod ui;

pub use args::*;
