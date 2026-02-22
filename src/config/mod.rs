//! Configuration Module
//!
//! Handles application configuration loading, validation, and management.

pub mod crabrace;
pub mod secrets;
mod types;
pub mod update;

pub use crabrace::{CrabraceConfig, CrabraceIntegration};
pub use secrets::SecretString;
pub use types::*;
pub use update::{ProviderUpdater, UpdateResult};
