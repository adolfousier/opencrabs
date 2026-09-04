//! Memory Module
//!
//! Provides long-term memory search via our own SQLite FTS5 store
//! (`db.rs`) and vector semantic search (embeddinggemma-300M, local GGUF
//! via `local_engine.rs` or an OpenAI-compatible embedding API). Hybrid
//! RRF when embeddings are available, FTS-only fallback otherwise.
//!
//! When `config.memory.vector_enabled` is false, all vector/embedding code
//! is skipped — no model download, no llama.cpp init, FTS5-only search.
//!
//! Layout: [`settings`] (the `[memory]` config readers), [`keys`] (keys.toml
//! embedding-key resolution), [`embedding_config`] (embedding API gates),
//! [`doctor`] (doctor-report lines), [`shared_sessions`] (the group-session
//! gate), [`types`] (result type + collection names); `db` / `store` /
//! `index` / `search` / `embedding` and the sweeps do the work. This file is
//! declarations only — no function definitions live here (CONTRIBUTING.md).

pub mod backfill_sweep;
pub(crate) mod chunk_fts;
pub(crate) mod chunker;
pub(crate) mod db;
pub(crate) mod doctor;
pub(crate) mod embedding;
pub(crate) mod embedding_config;
pub(crate) mod external;
pub(crate) mod external_sweep;
pub mod freshness;
pub mod health_report;
pub mod index;
pub(crate) mod keys;
pub(crate) mod local_engine;
pub(crate) mod search;
pub(crate) mod settings;
pub(crate) mod shared_sessions;
pub(crate) mod store;
#[cfg(feature = "code-graph")]
pub(crate) mod symbol_extractor;
pub(crate) mod types;
pub mod vector_search;

// The lib-consumed surface, re-exported so callers hold one path
// (`memory::doctor_lines`, `super::vector_enabled`, ...). Test-only consumers
// import from the source module directly (e.g. `memory::keys::`) so a
// lib-wide re-export can never go unused in the non-test target.
pub use db::Store;
pub use doctor::doctor_lines;
pub use embedding::{
    embed_content, embed_content_api, embed_query_api, embed_via_api, engine_if_ready, get_engine,
};
pub(crate) use embedding_config::{
    embedding_api_config, embedding_api_configured, embedding_dimensions,
};
pub use index::{BRAIN_FILES, index_file, index_file_fts_only, reindex};
#[cfg(feature = "code-graph")]
pub(crate) use search::search_symbol_graph;
pub use search::{RrfResult, hybrid_search_rrf, search, search_brain};
pub(crate) use search::{search_external, search_memory};
pub(crate) use settings::{
    external_allowed_in_shared, external_excludes, extra_paths_config, read_memory_config,
    sweep_interval_secs, vector_enabled,
};
pub use shared_sessions::{is_session_shared, mark_session_shared};
pub use store::get_store;
pub use types::MemoryResult;
pub(crate) use types::{COLLECTION_BRAIN, COLLECTION_EXTERNAL, COLLECTION_MEMORY};
