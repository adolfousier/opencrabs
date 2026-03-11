//! Memory Module
//!
//! Provides long-term memory search via the `qmd` crate's FTS5 engine and
//! vector semantic search (embeddinggemma-300M). Hybrid RRF when the model
//! is available, FTS-only fallback otherwise.

mod embedding;
mod index;
mod search;
mod store;

pub use embedding::{embed_content, engine_if_ready, get_engine};
pub use index::{BRAIN_FILES, index_file, reindex};
pub use search::search;
pub use store::get_store;

/// A single search result from the memory index.
#[derive(Debug, Clone)]
pub struct MemoryResult {
    pub path: String,
    pub snippet: String,
    pub rank: f64,
}

/// Collection name for daily compaction logs.
const COLLECTION_MEMORY: &str = "memory";
/// Collection name for workspace brain files (SOUL.md, MEMORY.md, etc.).
const COLLECTION_BRAIN: &str = "brain";
