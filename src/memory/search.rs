//! Search — hybrid FTS5 + vector search via Reciprocal Rank Fusion.

use super::db::{SearchResult, Store};
use std::path::Path;
use std::sync::Mutex;

use super::embedding::{embed_query_api, engine_if_ready};
use super::{
    COLLECTION_BRAIN, COLLECTION_EXTERNAL, COLLECTION_MEMORY, MemoryResult,
    embedding_api_configured,
};

/// Minimum effective cap for structural graph listings (#89): callers/callees/
/// definitions are rank-1 one-liners where the full set is the answer, so a
/// search-default n=5 would truncate arbitrarily. Applies to structural
/// listings only — never to ranked FTS/vector search paths.
#[cfg(feature = "code-graph")]
pub(crate) const MIN_STRUCTURAL_RESULTS: usize = 25;

/// Detect if a query is asking for structural code relationships.
///
/// Returns `Some((query_type, symbol))` where query_type is one of:
/// - "calls" - "who calls X", "what calls X"
/// - "called_by" - "what does X call", "what X calls"
/// - "implements" - "show implementations of X", "who implements X"
/// - "defined_in" - "where is X defined", "show definition of X"
///
/// Returns `None` for conceptual queries that should use BM25+vector.
#[cfg(feature = "code-graph")]
pub(crate) fn detect_structural_query(query: &str) -> Option<(String, String)> {
    let query_lower = query.to_lowercase();

    // "who calls X" / "what calls X" → find callers of X
    if let Some(caps) = regex::Regex::new(r"(?i)(who|what)\s+calls\s+([a-zA-Z_][a-zA-Z0-9_]*)")
        .ok()
        .and_then(|re| re.captures(&query_lower))
    {
        return Some(("calls".to_string(), caps[2].to_string()));
    }

    // "what does X call" / "what X calls" → find callees of X
    if let Some(caps) = regex::Regex::new(r"(?i)what\s+(?:does\s+)?([a-zA-Z_][a-zA-Z0-9_]*)\s+call")
        .ok()
        .and_then(|re| re.captures(&query_lower))
    {
        return Some(("called_by".to_string(), caps[1].to_string()));
    }

    // "show implementations of X" / "who implements X" → find impl blocks
    if let Some(caps) = regex::Regex::new(
        r"(?i)(?:show\s+)?(?:implementations?\s+of|who\s+implements)\s+([a-zA-Z_][a-zA-Z0-9_]*)",
    )
    .ok()
    .and_then(|re| re.captures(&query_lower))
    {
        return Some(("implements".to_string(), caps[1].to_string()));
    }

    // "where is X defined" / "show definition of X" → find symbol definition
    if let Some(caps) = regex::Regex::new(
        r"(?i)(?:where\s+is|show\s+(?:the\s+)?definition\s+of)\s+([a-zA-Z_][a-zA-Z0-9_]*)",
    )
    .ok()
    .and_then(|re| re.captures(&query_lower))
    {
        return Some(("defined_in".to_string(), caps[1].to_string()));
    }

    None
}

/// Execute a structural query against the symbol graph.
///
/// Structural listings ("who calls X") are RANK-1 result sets, not a ranked
/// search: every hit matters, so the search-default n=5 cap would truncate
/// the answer arbitrarily. The effective cap is floored at
/// [`MIN_STRUCTURAL_RESULTS`] here — and ONLY here. Ranked FTS/vector paths
/// keep the caller's requested `n` by design (Issue-Ref: leshchenko1979/opencrabs#89).
#[cfg(feature = "code-graph")]
fn search_symbol_graph(
    store: &Store,
    query_type: &str,
    symbol: &str,
    n: usize,
) -> Result<Vec<MemoryResult>, String> {
    let n = n.max(MIN_STRUCTURAL_RESULTS);
    match query_type {
        "calls" => {
            // Find who calls this symbol
            let callers = store.query_callers_of(symbol)?;
            Ok(callers
                .into_iter()
                .take(n)
                .map(|(caller, file_path, line)| MemoryResult {
                    path: file_path,
                    snippet: format!("{} calls {} at line {}", caller, symbol, line),
                    rank: 1.0,
                })
                .collect())
        }
        "called_by" => {
            // Find what this symbol calls
            let callees = store.query_callees_of(symbol)?;
            Ok(callees
                .into_iter()
                .take(n)
                .map(|(callee, file_path, line)| MemoryResult {
                    path: file_path,
                    snippet: format!("{} calls {} at line {}", symbol, callee, line),
                    rank: 1.0,
                })
                .collect())
        }
        "implements" | "defined_in" => {
            // Find symbol definitions
            let symbols = store.query_symbols_by_name(symbol)?;
            Ok(symbols
                .into_iter()
                .take(n)
                .map(|(kind, file_path, start_line, end_line)| MemoryResult {
                    path: file_path,
                    snippet: format!(
                        "{} {} defined at lines {}-{}",
                        kind, symbol, start_line, end_line
                    ),
                    rank: 1.0,
                })
                .collect())
        }
        _ => Ok(vec![]),
    }
}

/// Hybrid search across ALL collections in the store: FTS5 (BM25) + vector
/// (cosine) via RRF.
///
/// Kept collection-wide so existing callers (e.g. a2a debate context) see
/// everything. Scope-specific variants: `search_memory`, `search_brain`,
/// `search_external` (#1051).
///
/// Falls back to FTS-only when the embedding engine is unavailable.
/// Returns up to `n` results sorted by relevance.
pub async fn search(
    store: &'static Mutex<Store>,
    query: &str,
    n: usize,
) -> Result<Vec<MemoryResult>, String> {
    // Refresh brain files whose mtime moved since indexing (#1018). The index
    // was a boot-time snapshot, so a rule written mid-session was invisible
    // here until the next restart — precisely when a duplicate check needs it.
    // Stat-only for unchanged files, single-flight guarded, never fatal.
    super::freshness::refresh_stale_brain_files().await;
    search_core(store, query, n, None).await
}

/// Hybrid search over the memory-log collection only (#1051 scope="memory").
pub(crate) async fn search_memory(
    store: &'static Mutex<Store>,
    query: &str,
    n: usize,
) -> Result<Vec<MemoryResult>, String> {
    search_core(store, query, n, Some(COLLECTION_MEMORY)).await
}

/// Hybrid search over the EXTERNAL collection (#1051 scope="external").
///
/// Tier-1 freshness (ADR-002): after the first pass, refresh any hit whose
/// mtime moved and re-query once, so an in-place edit is reflected. The
/// tier-2 sweep handles additions and deletions; this catches content edits
/// that don't bump the parent directory's mtime.
///
/// Structural queries (code-graph feature): "who calls X", "what does X call",
/// etc. route to the symbol graph instead of BM25+vector search.
pub(crate) async fn search_external(
    store: &'static Mutex<Store>,
    query: &str,
    n: usize,
) -> Result<Vec<MemoryResult>, String> {
    // Check for structural query (code-graph feature)
    #[cfg(feature = "code-graph")]
    if let Some((query_type, symbol)) = detect_structural_query(query) {
        let store_lock = store
            .lock()
            .map_err(|e| format!("Store lock poisoned: {e}"))?;
        return search_symbol_graph(&store_lock, &query_type, &symbol, n);
    }

    let results = search_core(store, query, n, Some(COLLECTION_EXTERNAL)).await?;
    let paths: Vec<String> = results.iter().map(|r| r.path.clone()).collect();
    if super::freshness::refresh_stale_external(&paths).await > 0 {
        return search_core(store, query, n, Some(COLLECTION_EXTERNAL)).await;
    }
    Ok(results)
}

/// Core hybrid search over one collection, or all collections when
/// `collection` is `None`.
async fn search_core(
    store: &'static Mutex<Store>,
    query: &str,
    n: usize,
    collection: Option<&'static str>,
) -> Result<Vec<MemoryResult>, String> {
    let fts_query = sanitize_fts_query(query);
    if fts_query.is_empty() {
        return Ok(vec![]);
    }

    let query_owned = query.to_string();

    // API path: embed query via HTTP before entering spawn_blocking
    let api_embedding = if embedding_api_configured() {
        match embed_query_api(query).await {
            Ok(emb) => Some(emb),
            Err(e) => {
                tracing::warn!("API embedding failed for query, falling back to FTS-only: {e}");
                None
            }
        }
    } else {
        None
    };

    tokio::task::spawn_blocking(move || {
        // Local engine path: embed query via GGUF
        let query_embedding: Option<Vec<f32>> = if !embedding_api_configured() {
            engine_if_ready().and_then(|em| {
                em.lock()
                    .ok()
                    .and_then(|mut e| e.embed_query(&query_owned).ok().map(|r| r.embedding))
            })
        } else {
            api_embedding // Use the pre-fetched API embedding
        };

        // Store lock → search
        let store = store
            .lock()
            .map_err(|e| format!("Store lock poisoned: {e}"))?;
        let home = crate::config::opencrabs_home();

        let fts_results = store
            .search_fts(&fts_query, n, collection)
            .map_err(|e| format!("FTS search failed: {e}"))?;

        // Hybrid path: combine FTS + vector results via Reciprocal Rank Fusion
        if let Some(ref query_emb) = query_embedding {
            // Chunk-aware (#998): a vector search that joins on
            // `hash || '_0'` only ever sees a document's first chunk, which
            // would make chunked embeddings write-only.
            let db_path = super::store::memory_dir().join("memory.db");
            let vec_hits = super::vector_search::search_chunks(&db_path, query_emb, n, collection)
                .unwrap_or_default();

            if !vec_hits.is_empty() {
                let fts_tuples =
                    results_to_tuples_for(&store, &home, &fts_results, Some(&fts_query));
                let vec_tuples = chunk_hits_to_tuples(&store, &home, &vec_hits);
                let rrf = hybrid_search_rrf(fts_tuples, vec_tuples, 60);

                return Ok(rrf
                    .into_iter()
                    .take(n)
                    .map(|r| MemoryResult {
                        path: r.file,
                        snippet: extract_snippet(&r.body, &fts_query, 200),
                        rank: r.score,
                    })
                    .collect());
            }
        }

        // FTS-only fallback
        Ok(fts_results
            .iter()
            .map(|r| {
                let snippet = match store.get_document(&r.doc.collection_name, &r.doc.path) {
                    Ok(Some(doc)) => {
                        let body = doc.body.as_deref().unwrap_or("");
                        extract_snippet(body, &fts_query, 200)
                    }
                    _ => r.doc.title.clone(),
                };
                MemoryResult {
                    path: resolve_path(&home, &r.doc.collection_name, &r.doc.path),
                    snippet,
                    rank: r.score,
                }
            })
            .collect())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// FTS-only search over the brain-file collection (no vector overhead).
///
/// Sub-millisecond BM25 over the indexed brain files (SOUL/USER/AGENTS/TOOLS/
/// CODE/SECURITY/MEMORY/BOOT/HEARTBEAT). Used by the harness brain-hints layer
/// (#767) to inject relevant guidance into tool errors and `tool_search`
/// results without paying the embedding round-trip the hybrid `search` does.
pub async fn search_brain(
    store: &'static Mutex<Store>,
    query: &str,
    n: usize,
) -> Result<Vec<MemoryResult>, String> {
    // Refresh brain files whose mtime moved since indexing (#1018). The index
    // was a boot-time snapshot, so a rule written mid-session was invisible
    // here until the next restart — precisely when a duplicate check needs it.
    // Stat-only for unchanged files, single-flight guarded, never fatal.
    super::freshness::refresh_stale_brain_files().await;

    let fts_query = sanitize_fts_query(query);
    if fts_query.is_empty() {
        return Ok(vec![]);
    }

    tokio::task::spawn_blocking(move || {
        let store = store
            .lock()
            .map_err(|e| format!("Store lock poisoned: {e}"))?;
        let home = crate::config::opencrabs_home();

        let fts_results = store
            .search_fts(&fts_query, n, Some(COLLECTION_BRAIN))
            .map_err(|e| format!("FTS search failed: {e}"))?;

        Ok(fts_results
            .iter()
            .map(|r| {
                let snippet = match store.get_document(&r.doc.collection_name, &r.doc.path) {
                    Ok(Some(doc)) => {
                        let body = doc.body.as_deref().unwrap_or("");
                        extract_snippet(body, &fts_query, 200)
                    }
                    _ => r.doc.title.clone(),
                };
                MemoryResult {
                    path: resolve_path(&home, &r.doc.collection_name, &r.doc.path),
                    snippet,
                    rank: r.score,
                }
            })
            .collect())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Convert chunk hits to RRF tuple format: (file_path, display_path, title, body).
///
/// The BODY is the whole document, not the matching chunk. The chunk decided
/// WHICH document is relevant; the caller still snippets the full text, and
/// handing back only the chunk would lose the surrounding context that makes a
/// snippet readable.
fn chunk_hits_to_tuples(
    store: &Store,
    home: &Path,
    hits: &[super::vector_search::ChunkHit],
) -> Vec<(String, String, String, String)> {
    hits.iter()
        .map(|h| {
            let file_path = resolve_path(home, &h.collection, &h.path);
            let body = store
                .get_document(&h.collection, &h.path)
                .ok()
                .flatten()
                .and_then(|d| d.body)
                .unwrap_or_default();
            (file_path.clone(), file_path, h.title.clone(), body)
        })
        .collect()
}

/// Convert SearchResults to RRF tuple format: (file_path, display_path, title,
/// body), narrowing each hit to its best matching chunk when a query is given
/// (#1000).
///
/// `search_fts` matches whole documents, so without this the lexical half of
/// hybrid search ranks files while the vector half ranks chunks, and RRF fuses
/// two lists describing different units. Passing the query narrows the body to
/// the passage that earned the hit, which is also what the snippet should be
/// cut from.
fn results_to_tuples_for(
    store: &Store,
    home: &Path,
    results: &[SearchResult],
    query: Option<&str>,
) -> Vec<(String, String, String, String)> {
    results
        .iter()
        .map(|r| {
            let file_path = resolve_path(home, &r.doc.collection_name, &r.doc.path);
            let full = store
                .get_document(&r.doc.collection_name, &r.doc.path)
                .ok()
                .flatten()
                .and_then(|d| d.body)
                .unwrap_or_default();
            let body = match query.and_then(|q| super::chunk_fts::best_chunk(&full, q)) {
                Some((_, chunk)) => chunk,
                None => full,
            };
            (
                file_path,
                r.doc.display_path.clone(),
                r.doc.title.clone(),
                body,
            )
        })
        .collect()
}

/// Resolve filesystem path for a search result based on its collection.
fn resolve_path(home: &Path, collection: &str, doc_path: &str) -> String {
    if collection == COLLECTION_EXTERNAL {
        // External documents are keyed by ABSOLUTE path (#1051): the stored
        // path IS the filesystem path. Rebuilding it as home/memory/<key>
        // would point every external hit at a nonexistent file, so pass it
        // through unchanged (mine map #3).
        return doc_path.to_string();
    }
    let p = if collection == COLLECTION_BRAIN {
        home.join(doc_path)
    } else {
        home.join("memory").join(doc_path)
    };
    p.to_string_lossy().to_string()
}

/// Sanitize a search query for FTS5: wrap each word in double quotes
/// to avoid syntax errors from special characters, then join with spaces (implicit AND).
pub(crate) fn sanitize_fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|w| {
            let clean: String = w.chars().filter(|c| *c != '"').collect();
            format!("\"{clean}\"")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Extract a snippet from body text around the first query term match.
pub(crate) fn extract_snippet(body: &str, query: &str, max_len: usize) -> String {
    let query_lower = query.to_lowercase();
    let body_lower = body.to_lowercase();

    let mut best_pos = 0;
    for word in query_lower.split_whitespace() {
        let clean: String = word.chars().filter(|c| *c != '"').collect();
        if !clean.is_empty()
            && let Some(pos) = body_lower.find(&clean)
        {
            best_pos = pos;
            break;
        }
    }

    let start = best_pos.saturating_sub(50);
    let end = (start + max_len).min(body.len());

    let start = body.floor_char_boundary(start);
    let end = body.ceil_char_boundary(end);

    let mut snippet = String::new();
    if start > 0 {
        snippet.push_str("...");
    }
    snippet.push_str(body[start..end].trim());
    if end < body.len() {
        snippet.push_str("...");
    }

    snippet
}

// ---------------------------------------------------------------------------
// Reciprocal Rank Fusion
// ---------------------------------------------------------------------------

/// One fused hybrid-search hit.
pub struct RrfResult {
    pub file: String,
    pub display_path: String,
    pub title: String,
    pub body: String,
    pub score: f64,
}

/// Combine two ranked lists (FTS, vector) with Reciprocal Rank Fusion.
///
/// RRF score = sum(weight / (k + rank + 1)) across the lists a document
/// appears in; k=60 is the standard balance between top and lower ranks.
/// Position-aware bonuses protect top retrieval results from disagreement
/// between the two halves: rank 1-3 get 0.08, rank 4-10 get 0.04, rank
/// 11-20 get 0.01. These numbers are ranking behavior, not decoration —
/// they shipped with the first hybrid search and stayed.
pub fn hybrid_search_rrf(
    fts_results: Vec<(String, String, String, String)>,
    vec_results: Vec<(String, String, String, String)>,
    k: usize,
) -> Vec<RrfResult> {
    use std::collections::HashMap;

    let mut scores: HashMap<String, (f64, String, String, String, usize)> = HashMap::new();

    for results in [fts_results, vec_results] {
        for (rank, (file, display_path, title, body)) in results.iter().enumerate() {
            let rrf_score = 1.0 / (k + rank + 1) as f64;

            scores
                .entry(file.clone())
                .and_modify(|(score, _, _, _, best_rank)| {
                    *score += rrf_score;
                    *best_rank = (*best_rank).min(rank);
                })
                .or_insert((
                    rrf_score,
                    display_path.clone(),
                    title.clone(),
                    body.clone(),
                    rank,
                ));
        }
    }

    let mut results: Vec<RrfResult> = scores
        .into_iter()
        .map(|(file, (score, display_path, title, body, best_rank))| {
            let bonus = match best_rank {
                0..=2 => 0.08,   // Top 3: high protection
                3..=9 => 0.04,   // Rank 4-10: medium protection
                10..=19 => 0.01, // Rank 11-20: low protection
                _ => 0.0,
            };

            RrfResult {
                file,
                display_path,
                title,
                body,
                score: score + bonus,
            }
        })
        .collect();

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    results
}
