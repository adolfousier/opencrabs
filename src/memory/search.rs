//! Search — hybrid FTS5 + vector search via Reciprocal Rank Fusion.

use qmd::{SearchResult, Store, hybrid_search_rrf};
use std::path::Path;
use std::sync::Mutex;

use super::embedding::engine_if_ready;
use super::{COLLECTION_BRAIN, MemoryResult};

/// Hybrid search across memory logs: FTS5 (BM25) + vector (cosine) via RRF.
///
/// Falls back to FTS-only when the embedding engine is unavailable.
/// Returns up to `n` results sorted by relevance.
pub async fn search(
    store: &'static Mutex<Store>,
    query: &str,
    n: usize,
) -> Result<Vec<MemoryResult>, String> {
    let fts_query = sanitize_fts_query(query);
    if fts_query.is_empty() {
        return Ok(vec![]);
    }

    let query_owned = query.to_string();

    tokio::task::spawn_blocking(move || {
        // Engine lock → embed query → release (before store lock)
        let query_embedding: Option<Vec<f32>> = engine_if_ready().and_then(|em| {
            em.lock()
                .ok()
                .and_then(|mut e| e.embed_query(&query_owned).ok().map(|r| r.embedding))
        });

        // Store lock → search
        let store = store
            .lock()
            .map_err(|e| format!("Store lock poisoned: {e}"))?;
        let home = crate::config::opencrabs_home();

        let fts_results = store
            .search_fts(&fts_query, n, None)
            .map_err(|e| format!("FTS search failed: {e}"))?;

        // Hybrid path: combine FTS + vector results via Reciprocal Rank Fusion
        if let Some(ref query_emb) = query_embedding {
            let vec_results = store.search_vec(query_emb, n, None).unwrap_or_default();

            if !vec_results.is_empty() {
                let fts_tuples = results_to_tuples(&store, &home, &fts_results);
                let vec_tuples = results_to_tuples(&store, &home, &vec_results);
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

/// Convert SearchResults to RRF tuple format: (file_path, display_path, title, body).
fn results_to_tuples(
    store: &Store,
    home: &Path,
    results: &[SearchResult],
) -> Vec<(String, String, String, String)> {
    results
        .iter()
        .map(|r| {
            let file_path = resolve_path(home, &r.doc.collection_name, &r.doc.path);
            let body = store
                .get_document(&r.doc.collection_name, &r.doc.path)
                .ok()
                .flatten()
                .and_then(|d| d.body)
                .unwrap_or_default();
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
    let p = if collection == COLLECTION_BRAIN {
        home.join(doc_path)
    } else {
        home.join("memory").join(doc_path)
    };
    p.to_string_lossy().to_string()
}

/// Sanitize a search query for FTS5: wrap each word in double quotes
/// to avoid syntax errors from special characters, then join with spaces (implicit AND).
fn sanitize_fts_query(query: &str) -> String {
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
fn extract_snippet(body: &str, query: &str, max_len: usize) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_fts_query() {
        assert_eq!(sanitize_fts_query("hello world"), "\"hello\" \"world\"");
        assert_eq!(sanitize_fts_query(""), "");
        assert_eq!(sanitize_fts_query("auth\"bug"), "\"authbug\"");
    }

    #[test]
    fn test_extract_snippet() {
        let body = "# Today\nFixed the authentication bug in login flow. Also refactored database.";
        let snippet = extract_snippet(body, "\"authentication\"", 60);
        assert!(snippet.contains("authentication"));
    }

    #[test]
    fn test_extract_snippet_no_match() {
        let body = "Some content without the search term";
        let snippet = extract_snippet(body, "\"nonexistent\"", 60);
        assert!(snippet.contains("Some content"));
    }
}
