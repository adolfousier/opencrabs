//! Evolve Tool
//!
//! Updates OpenCrabs to the latest release. Detects the install method
//! (pre-built binary, cargo install, or source build) and uses the
//! appropriate upgrade strategy:
//!
//! - **Pre-built binary**: Downloads from GitHub releases, health-checks, swaps.
//! - **cargo install**: Runs `cargo install opencrabs --force`.
//! - **Source build**: Suggests rebuilding from the source tree instead.
//!
//! Before swapping binaries, it health-checks the new binary. If the swap
//! fails, it rolls back to the previous version automatically.
//!
//! Layout: [`tool`] (the `Tool` impl and dispatch), [`release_check`]
//! (GitHub release probe + version compare), [`binary_health`] (pre-swap
//! probes), [`via_binary_download`] / [`via_cargo_install`] (the two
//! strategies), [`restart_status`], [`archive`], [`systemd`], [`homebrew`].
//! This file is declarations only — no function definitions live here
//! (CONTRIBUTING.md).

mod archive;
mod binary_health;
pub(crate) mod bounded_child;
pub(crate) mod homebrew;
pub(crate) mod release_check;
pub(crate) mod restart_status;
pub(crate) mod systemd;
mod tool;
pub(crate) mod verify;
mod via_binary_download;
mod via_cargo_install;

pub use release_check::{check_for_update, is_newer};
pub use tool::EvolveTool;
