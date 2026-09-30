//! `[memory]` config readers.
//!
//! Every reader parses config.toml fresh on each call — the module live-reads
//! config by construction, so a change settles within one sweep interval with
//! no restart or reload handler.

use super::keys::{memory_embedding_key_from_keys_file, needs_embedding_key};
use crate::config::{ExtraPath, MemoryConfig};

/// Whether vector embeddings are enabled in the current config.
/// Reads `[memory].vector_enabled` from config.toml (default: false, the
/// opt-in flipped in #1798). VPS/cloud auto-detection additionally writes
/// an explicit false at startup.
pub(crate) fn vector_enabled() -> bool {
    let config = read_memory_config();
    config.vector_enabled
}

/// Read the `[memory]` section from config.toml, resolving the embedding
/// `api_key` from keys.toml when config.toml does not carry one.
///
/// `EmbeddingConfig::api_key` has always documented that it is "also loaded
/// from keys.toml under `[providers.memory_embedding]`", but nothing
/// implemented it, and the gap was two layers deep (#1066): `ProviderConfigs`
/// had no `memory_embedding` field, so serde dropped the section outright —
/// and even with a merge arm it could not have helped, because this function
/// parses config.toml directly and never passes through `Config::load()`.
/// Resolving the key here fixes it at the one place that actually consumes it,
/// and keeps the secret in keys.toml where the config/keys split intends it.
pub(crate) fn read_memory_config() -> MemoryConfig {
    let mut cfg = read_memory_section().unwrap_or_default();
    if let Some(ref mut emb) = cfg.embedding
        && needs_embedding_key(emb.api_key.as_deref())
        && let Some(key) = memory_embedding_key_from_keys_file()
    {
        emb.api_key = Some(key);
    }
    cfg
}

/// Parse just the `[memory]` table out of config.toml.
pub(super) fn read_memory_section() -> Option<MemoryConfig> {
    let config_path = crate::config::opencrabs_home().join("config.toml");
    let content = std::fs::read_to_string(&config_path).ok()?;
    let table = content.parse::<toml::Table>().ok()?;
    let memory = table.get("memory")?;
    toml::from_str::<MemoryConfig>(&toml::to_string(memory).ok()?).ok()
}

/// Extra external paths to index, from `[memory].extra_paths` (#1051).
/// Read fresh on every call — config changes settle within one sweep
/// interval with no restart or reload handler (the module live-reads
/// config by construction).
pub(crate) fn extra_paths_config() -> Vec<ExtraPath> {
    read_memory_config().extra_paths
}

/// Global exclude patterns for external indexing (#1051).
pub(crate) fn external_excludes() -> Vec<String> {
    read_memory_config().exclude
}

/// Whether external results may surface in shared/group sessions (#1051).
/// Defaults to deny — the session gate is the security boundary.
pub(crate) fn external_allowed_in_shared() -> bool {
    read_memory_config().external_allowed_in_shared
}

/// Seconds between external freshness sweeps (#1051).
pub(crate) fn sweep_interval_secs() -> u64 {
    read_memory_config().sweep_interval_secs
}
