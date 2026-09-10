//! Store — the memory database, owned by OpenCrabs.
//!
//! This replaced the `qmd` crate (qntx/qmd 0.3.2), which had not published a
//! release since Feb 2026 while every defect it ever shipped was found and
//! fixed on our side: byte-index chunking that panicked on multi-byte chars
//! (#1002), vector search that only ever saw chunk 0 (#998), doc-level-only
//! FTS (#1000). The audit (2026-08-12) mapped the entire surface we used —
//! 13 Store methods — and this file is that surface, nothing more: no rerank,
//! no generation, no collections admin, no llm_cache.
//!
//! The schema is qmd's verbatim on purpose. Every existing `memory.db` in the
//! wild was created by qmd, and `vector_search.rs` reads these tables
//! directly, so the DDL below IS the migration story: there isn't one.

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// One indexed document row plus its body when loaded.
#[derive(Debug, Clone)]
pub struct DocumentResult {
    pub collection_name: String,
    pub path: String,
    pub display_path: String,
    pub title: String,
    pub hash: String,
    pub modified_at: String,
    pub body: Option<String>,
}

/// A scored search hit.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub doc: DocumentResult,
    pub score: f64,
}

/// How much of the store is vectorised (#1067).
///
/// `documents_unembedded` and `last_embedded_at` are the two numbers that make
/// a stalled backfill visible: a store with 65 of 66 documents waiting and a
/// last-embedded date three months old is broken, and nothing said so before.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VectorStats {
    /// Active documents in the index.
    pub documents_active: usize,
    /// Active documents with no vector row at all.
    pub documents_unembedded: usize,
    /// Rows in `content_vectors`, one per embedded chunk.
    pub vector_rows: usize,
    /// UTC timestamp of the most recent embedding, if any.
    pub last_embedded_at: Option<String>,
}

/// The database store: one connection per database file, owned by the caller
/// behind a Mutex (see `super::store`).
#[derive(Debug)]
pub struct Store {
    conn: Connection,
    db_path: PathBuf,
}

impl Store {
    /// Open (or create) the store at `db_path` and initialize the schema.
    pub fn open(db_path: &Path) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create store dir: {e}"))?;
        }

        let conn = Connection::open(db_path)
            .map_err(|e| format!("Failed to open {}: {e}", db_path.display()))?;

        // WAL lets `vector_search` read on its own read-only connection
        // while this one writes. The busy timeout covers the rare writer/
        // snapshot overlap without changing any observable behavior.
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| format!("Failed to set busy timeout: {e}"))?;

        let mut store = Self {
            conn,
            db_path: db_path.to_path_buf(),
        };
        store.initialize()?;
        Ok(store)
    }

    /// Database file path.
    #[must_use]
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Schema DDL, byte-for-byte the one qmd 0.3.2 created. Do not "improve"
    /// it casually: existing memory.db files and vector_search.rs depend on
    /// exactly these tables, columns and triggers.
    fn initialize(&mut self) -> Result<(), String> {
        self.conn
            .execute_batch(
                r"
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;

            -- Content-addressable storage
            CREATE TABLE IF NOT EXISTS content (
                hash TEXT PRIMARY KEY,
                doc TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            -- Documents table
            CREATE TABLE IF NOT EXISTS documents (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                collection TEXT NOT NULL,
                path TEXT NOT NULL,
                title TEXT NOT NULL,
                hash TEXT NOT NULL,
                created_at TEXT NOT NULL,
                modified_at TEXT NOT NULL,
                active INTEGER NOT NULL DEFAULT 1,
                FOREIGN KEY (hash) REFERENCES content(hash) ON DELETE CASCADE,
                UNIQUE(collection, path)
            );

            CREATE INDEX IF NOT EXISTS idx_documents_collection ON documents(collection, active);
            CREATE INDEX IF NOT EXISTS idx_documents_hash ON documents(hash);
            CREATE INDEX IF NOT EXISTS idx_documents_path ON documents(path, active);

            -- FTS index
            CREATE VIRTUAL TABLE IF NOT EXISTS documents_fts USING fts5(
                filepath, title, body,
                tokenize='porter unicode61'
            );

            -- Content vectors metadata
            CREATE TABLE IF NOT EXISTS content_vectors (
                hash TEXT NOT NULL,
                seq INTEGER NOT NULL DEFAULT 0,
                pos INTEGER NOT NULL DEFAULT 0,
                model TEXT NOT NULL,
                embedded_at TEXT NOT NULL,
                chunk_hash TEXT,
                PRIMARY KEY (hash, seq)
            );
            ",
            )
            .map_err(|e| format!("Failed to initialize schema: {e}"))?;

        // Column heal (#14): the DDL above is CREATE TABLE IF NOT EXISTS, a
        // no-op on tables created by older builds. A content_vectors table
        // that predates #1107 (2026-08-19) has no chunk_hash column, and
        // chunk_needs_embedding then fails on every chunk of every backfill
        // cycle. Probe and add the column when it is missing; idempotent.
        self.ensure_chunk_hash_column()?;

        self.create_fts_triggers()
    }

    /// Add `chunk_hash` to `content_vectors` when the table predates it (#14).
    ///
    /// SQLite has no `ADD COLUMN IF NOT EXISTS`, so probe `pragma_table_info`
    /// and ALTER only when the column is absent. On a fresh store the DDL
    /// above already created the column and this stays a single read.
    fn ensure_chunk_hash_column(&self) -> Result<(), String> {
        let has_column: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('content_vectors')
                 WHERE name = 'chunk_hash'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .map_err(|e| format!("chunk_hash column probe: {e}"))?;

        if !has_column {
            self.conn
                .execute_batch("ALTER TABLE content_vectors ADD COLUMN chunk_hash TEXT;")
                .map_err(|e| format!("chunk_hash column heal: {e}"))?;
        }
        Ok(())
    }

    /// FTS5 external-content sync triggers. Copied verbatim from qmd: the
    /// FTS index is keyed by documents.id (rowid) and mirrors active rows.
    fn create_fts_triggers(&self) -> Result<(), String> {
        let trigger_exists: bool = self
            .conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='trigger' AND name='documents_ai'",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);

        if !trigger_exists {
            self.conn
                .execute_batch(
                    r"
                CREATE TRIGGER IF NOT EXISTS documents_ai AFTER INSERT ON documents
                WHEN new.active = 1
                BEGIN
                    INSERT INTO documents_fts(rowid, filepath, title, body)
                    SELECT
                        new.id,
                        new.collection || '/' || new.path,
                        new.title,
                        (SELECT doc FROM content WHERE hash = new.hash)
                    WHERE new.active = 1;
                END;

                CREATE TRIGGER IF NOT EXISTS documents_ad AFTER DELETE ON documents BEGIN
                    DELETE FROM documents_fts WHERE rowid = old.id;
                END;

                CREATE TRIGGER IF NOT EXISTS documents_au AFTER UPDATE ON documents
                BEGIN
                    DELETE FROM documents_fts WHERE rowid = old.id AND new.active = 0;
                    INSERT OR REPLACE INTO documents_fts(rowid, filepath, title, body)
                    SELECT
                        new.id,
                        new.collection || '/' || new.path,
                        new.title,
                        (SELECT doc FROM content WHERE hash = new.hash)
                    WHERE new.active = 1;
                END;
                ",
                )
                .map_err(|e| format!("Failed to create FTS triggers: {e}"))?;
        }

        Ok(())
    }

    /// Hash content using SHA256. Must stay byte-compatible with hashes
    /// already stored (they are the foreign keys into `content`).
    #[must_use]
    pub fn hash_content(content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Extract title from markdown content (first `#` or `##` heading).
    #[must_use]
    pub fn extract_title(content: &str) -> String {
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("# ") {
                return rest.trim().to_string();
            }
            if let Some(rest) = trimmed.strip_prefix("## ") {
                return rest.trim().to_string();
            }
        }
        String::new()
    }

    /// Insert content into content-addressable storage.
    pub fn insert_content(
        &self,
        hash: &str,
        content: &str,
        created_at: &str,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO content (hash, doc, created_at) VALUES (?1, ?2, ?3)",
                params![hash, content, created_at],
            )
            .map_err(|e| format!("insert_content: {e}"))?;
        Ok(())
    }

    /// Insert (or reactivate/update) a document record.
    pub fn insert_document(
        &self,
        collection: &str,
        path: &str,
        title: &str,
        hash: &str,
        created_at: &str,
        modified_at: &str,
    ) -> Result<(), String> {
        self.conn
            .execute(
                r"
            INSERT INTO documents (collection, path, title, hash, created_at, modified_at, active)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)
            ON CONFLICT(collection, path) DO UPDATE SET
                title = excluded.title,
                hash = excluded.hash,
                modified_at = excluded.modified_at,
                active = 1
            ",
                params![collection, path, title, hash, created_at, modified_at],
            )
            .map_err(|e| format!("insert_document: {e}"))?;
        Ok(())
    }

    /// Find an active document by collection and path.
    pub fn find_active_document(
        &self,
        collection: &str,
        path: &str,
    ) -> Result<Option<(i64, String, String)>, String> {
        self.conn
            .query_row(
                "SELECT id, hash, title FROM documents WHERE collection = ?1 AND path = ?2 AND active = 1",
                params![collection, path],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|e| format!("find_active_document: {e}"))
    }

    /// Deactivate a document (soft delete; the FTS trigger drops the row).
    pub fn deactivate_document(&self, collection: &str, path: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE documents SET active = 0 WHERE collection = ?1 AND path = ?2",
                params![collection, path],
            )
            .map_err(|e| format!("deactivate_document: {e}"))?;
        Ok(())
    }

    /// All active document paths in a collection.
    pub fn get_active_document_paths(&self, collection: &str) -> Result<Vec<String>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM documents WHERE collection = ?1 AND active = 1")
            .map_err(|e| format!("get_active_document_paths: {e}"))?;
        let paths = stmt
            .query_map(params![collection], |row| row.get(0))
            .map_err(|e| format!("get_active_document_paths: {e}"))?
            .collect::<Result<Vec<String>, _>>()
            .map_err(|e| format!("get_active_document_paths: {e}"))?;
        Ok(paths)
    }

    /// Load one active document with its full body.
    pub fn get_document(
        &self,
        collection: &str,
        path: &str,
    ) -> Result<Option<DocumentResult>, String> {
        self.conn
            .query_row(
                r"
                SELECT
                    d.title,
                    d.hash,
                    d.modified_at,
                    c.doc,
                    LENGTH(c.doc) as body_length
                FROM documents d
                JOIN content c ON c.hash = d.hash
                WHERE d.collection = ?1 AND d.path = ?2 AND d.active = 1
                ",
                params![collection, path],
                |row| {
                    let title: String = row.get(0)?;
                    let hash: String = row.get(1)?;
                    let modified_at: String = row.get(2)?;
                    let body: String = row.get(3)?;

                    Ok(DocumentResult {
                        collection_name: collection.to_string(),
                        path: path.to_string(),
                        display_path: format!("{collection}/{path}"),
                        title,
                        hash,
                        modified_at,
                        body: Some(body),
                    })
                },
            )
            .optional()
            .map_err(|e| format!("get_document: {e}"))
    }

    /// Full-text search (BM25) over active documents, optionally restricted
    /// to one collection. Scores are negated bm25(): higher is better.
    pub fn search_fts(
        &self,
        query: &str,
        limit: usize,
        collection: Option<&str>,
    ) -> Result<Vec<SearchResult>, String> {
        let sql = if collection.is_some() {
            r"
            SELECT
                d.collection,
                d.path,
                d.title,
                d.hash,
                d.modified_at,
                bm25(documents_fts) as score
            FROM documents_fts fts
            JOIN documents d ON d.id = fts.rowid
            JOIN content c ON c.hash = d.hash
            WHERE documents_fts MATCH ?1
              AND d.collection = ?2
              AND d.active = 1
            ORDER BY score
            LIMIT ?3
            "
        } else {
            r"
            SELECT
                d.collection,
                d.path,
                d.title,
                d.hash,
                d.modified_at,
                bm25(documents_fts) as score
            FROM documents_fts fts
            JOIN documents d ON d.id = fts.rowid
            JOIN content c ON c.hash = d.hash
            WHERE documents_fts MATCH ?1
              AND d.active = 1
            ORDER BY score
            LIMIT ?2
            "
        };

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| format!("search_fts prepare: {e}"))?;

        let map_row = |row: &rusqlite::Row| -> rusqlite::Result<SearchResult> {
            let collection_name: String = row.get(0)?;
            let path: String = row.get(1)?;
            let title: String = row.get(2)?;
            let hash: String = row.get(3)?;
            let modified_at: String = row.get(4)?;
            let score: f64 = row.get(5)?;

            Ok(SearchResult {
                doc: DocumentResult {
                    collection_name: collection_name.clone(),
                    display_path: format!("{collection_name}/{path}"),
                    path: path.clone(),
                    title,
                    hash,
                    modified_at,
                    body: None,
                },
                // BM25 returns negative scores; negate so higher is better.
                score: -score,
            })
        };

        let results: Vec<SearchResult> = if let Some(coll) = collection {
            stmt.query_map(params![query, coll, limit as i64], map_row)
        } else {
            stmt.query_map(params![query, limit as i64], map_row)
        }
        .map_err(|e| format!("search_fts: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("search_fts: {e}"))?;

        Ok(results)
    }

    /// Create the vector blob table (only when embeddings are enabled).
    /// Dimensions are not enforced in the schema; rows carry whatever the
    /// configured backend produced.
    pub fn ensure_vector_table(&self, _dimensions: usize) -> Result<(), String> {
        self.conn
            .execute(
                r"
                CREATE TABLE IF NOT EXISTS vectors_vec (
                    hash_seq TEXT PRIMARY KEY,
                    embedding BLOB NOT NULL
                )
                ",
                [],
            )
            .map_err(|e| format!("ensure_vector_table: {e}"))?;
        Ok(())
    }

    /// Create symbol graph tables for code-graph feature.
    ///
    /// Three tables:
    /// - `symbols`: function/struct/enum/trait/impl/import definitions
    /// - `call_edges`: caller → callee relationships
    /// - `imports`: module import paths
    #[cfg(feature = "code-graph")]
    pub fn ensure_symbol_tables(&self) -> Result<(), String> {
        self.conn
            .execute_batch(
                r"
                CREATE TABLE IF NOT EXISTS symbols (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    symbol_name TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    file_path TEXT NOT NULL,
                    start_line INTEGER NOT NULL,
                    end_line INTEGER NOT NULL,
                    indexed_at TEXT NOT NULL
                );

                CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(symbol_name);
                CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file_path);
                CREATE INDEX IF NOT EXISTS idx_symbols_kind ON symbols(kind);

                CREATE TABLE IF NOT EXISTS call_edges (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    caller_symbol TEXT NOT NULL,
                    callee_symbol TEXT NOT NULL,
                    file_path TEXT NOT NULL,
                    call_line INTEGER NOT NULL,
                    indexed_at TEXT NOT NULL
                );

                CREATE INDEX IF NOT EXISTS idx_call_edges_caller ON call_edges(caller_symbol);
                CREATE INDEX IF NOT EXISTS idx_call_edges_callee ON call_edges(callee_symbol);

                CREATE TABLE IF NOT EXISTS imports (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    module_path TEXT NOT NULL,
                    file_path TEXT NOT NULL,
                    import_line INTEGER NOT NULL,
                    indexed_at TEXT NOT NULL
                );

                CREATE INDEX IF NOT EXISTS idx_imports_module ON imports(module_path);
                CREATE INDEX IF NOT EXISTS idx_imports_file ON imports(file_path);
                ",
            )
            .map_err(|e| format!("ensure_symbol_tables: {e}"))?;
        Ok(())
    }

    /// Row counts for the symbol graph tables: (symbols, call_edges, imports).
    /// Used by logging and benchmark scaffolding.
    #[cfg(feature = "code-graph")]
    pub fn symbol_graph_counts(&self) -> Result<(i64, i64, i64), String> {
        let symbols: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
            .map_err(|e| format!("symbol_graph_counts symbols: {e}"))?;
        let edges: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM call_edges", [], |r| r.get(0))
            .map_err(|e| format!("symbol_graph_counts call_edges: {e}"))?;
        let imports: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM imports", [], |r| r.get(0))
            .map_err(|e| format!("symbol_graph_counts imports: {e}"))?;
        Ok((symbols, edges, imports))
    }

    /// Insert a symbol into the symbol graph.
    #[cfg(feature = "code-graph")]
    pub fn insert_symbol(
        &self,
        symbol_name: &str,
        kind: &str,
        file_path: &str,
        start_line: usize,
        end_line: usize,
    ) -> Result<(), String> {
        let indexed_at = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                r"
                INSERT INTO symbols (symbol_name, kind, file_path, start_line, end_line, indexed_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ",
                params![
                    symbol_name,
                    kind,
                    file_path,
                    start_line as i64,
                    end_line as i64,
                    indexed_at
                ],
            )
            .map_err(|e| format!("insert_symbol: {e}"))?;
        Ok(())
    }

    /// Insert a call edge (caller → callee).
    #[cfg(feature = "code-graph")]
    pub fn insert_call_edge(
        &self,
        caller_symbol: &str,
        callee_symbol: &str,
        file_path: &str,
        call_line: usize,
    ) -> Result<(), String> {
        let indexed_at = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                r"
                INSERT INTO call_edges (caller_symbol, callee_symbol, file_path, call_line, indexed_at)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ",
                params![
                    caller_symbol,
                    callee_symbol,
                    file_path,
                    call_line as i64,
                    indexed_at
                ],
            )
            .map_err(|e| format!("insert_call_edge: {e}"))?;
        Ok(())
    }

    /// Insert an import.
    #[cfg(feature = "code-graph")]
    pub fn insert_import(
        &self,
        module_path: &str,
        file_path: &str,
        import_line: usize,
    ) -> Result<(), String> {
        let indexed_at = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                r"
                INSERT INTO imports (module_path, file_path, import_line, indexed_at)
                VALUES (?1, ?2, ?3, ?4)
                ",
                params![module_path, file_path, import_line as i64, indexed_at],
            )
            .map_err(|e| format!("insert_import: {e}"))?;
        Ok(())
    }

    /// Query symbols by name (exact match).
    #[cfg(feature = "code-graph")]
    pub fn query_symbols_by_name(
        &self,
        name: &str,
    ) -> Result<Vec<(String, String, usize, usize)>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r"
                SELECT kind, file_path, start_line, end_line
                FROM symbols
                WHERE symbol_name = ?1
                ORDER BY (file_path LIKE '%/tests/%'), kind, file_path, start_line
                ",
            )
            .map_err(|e| format!("query_symbols_by_name prepare: {e}"))?;

        let rows = stmt
            .query_map(params![name], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? as usize,
                    row.get::<_, i64>(3)? as usize,
                ))
            })
            .map_err(|e| format!("query_symbols_by_name: {e}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("query_symbols_by_name collect: {e}"))
    }

    /// Query call edges where the given symbol is the callee (who calls this function?).
    #[cfg(feature = "code-graph")]
    pub fn query_callers_of(&self, callee: &str) -> Result<Vec<(String, String, usize)>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r"
                SELECT caller_symbol, file_path, call_line
                FROM call_edges
                WHERE callee_symbol = ?1
                ORDER BY caller_symbol, file_path, call_line
                ",
            )
            .map_err(|e| format!("query_callers_of prepare: {e}"))?;

        let rows = stmt
            .query_map(params![callee], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? as usize,
                ))
            })
            .map_err(|e| format!("query_callers_of: {e}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("query_callers_of collect: {e}"))
    }

    /// Query call edges where the given symbol is the caller (what does this function call?).
    #[cfg(feature = "code-graph")]
    pub fn query_callees_of(&self, caller: &str) -> Result<Vec<(String, String, usize)>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r"
                SELECT callee_symbol, file_path, call_line
                FROM call_edges
                WHERE caller_symbol = ?1
                ORDER BY callee_symbol, file_path, call_line
                ",
            )
            .map_err(|e| format!("query_callees_of prepare: {e}"))?;

        let rows = stmt
            .query_map(params![caller], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? as usize,
                ))
            })
            .map_err(|e| format!("query_callees_of: {e}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("query_callees_of collect: {e}"))
    }

    /// Transitive callers of a symbol up to `max_depth` hops, via a recursive
    /// walk of `call_edges` (#89). Returns (caller, file, call_line, depth)
    /// with depth 1 = direct caller; each caller appears once at its SHALLOWEST
    /// depth, deterministically ordered by (depth, caller, file, line).
    ///
    /// Cycle guard: the recursive member increments `depth` and is bounded by
    /// `imp.depth < ?2`, so an A→B→A cycle in corrupt data terminates at the
    /// depth cap instead of walking forever; UNION (not UNION ALL) keeps the
    /// intermediate row set from fanning out on repeated identical edges.
    #[cfg(feature = "code-graph")]
    pub fn query_transitive_callers(
        &self,
        callee: &str,
        max_depth: usize,
    ) -> Result<Vec<(String, String, usize, usize)>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r"
                SELECT caller_symbol, file_path, call_line, MIN(depth) AS depth
                FROM (
                    WITH RECURSIVE impact(caller_symbol, file_path, call_line, depth) AS (
                        SELECT caller_symbol, file_path, call_line, 1
                        FROM call_edges
                        WHERE callee_symbol = ?1
                        UNION
                        SELECT ce.caller_symbol, ce.file_path, ce.call_line, imp.depth + 1
                        FROM call_edges ce
                        JOIN impact imp ON ce.callee_symbol = imp.caller_symbol
                        WHERE imp.depth < ?2
                    )
                    SELECT caller_symbol, file_path, call_line, depth FROM impact
                )
                GROUP BY caller_symbol
                ORDER BY depth, caller_symbol, file_path, call_line
                ",
            )
            .map_err(|e| format!("query_transitive_callers prepare: {e}"))?;

        let rows = stmt
            .query_map(params![callee, max_depth as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? as usize,
                    row.get::<_, i64>(3)? as usize,
                ))
            })
            .map_err(|e| format!("query_transitive_callers: {e}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("query_transitive_callers collect: {e}"))
    }

    /// Insert (or replace) an embedding for one chunk of a content hash.
    /// `hash_seq` is `{hash}_{seq}` — the key format `vector_search.rs`
    /// reads, so it must not change.
    /// Insert an embedding vector with per-chunk hash for caching (#1107).
    ///
    /// `hash` is the hash of the WHOLE document (foreign key to `content` table).
    /// `seq` is the chunk sequence number within the document.
    /// `chunk_hash` is the hash of THIS chunk's content (for cache invalidation).
    #[allow(clippy::too_many_arguments)]
    pub fn insert_embedding(
        &self,
        hash: &str,
        seq: usize,
        pos: usize,
        embedding: &[f32],
        model: &str,
        embedded_at: &str,
        chunk_hash: Option<&str>,
    ) -> Result<(), String> {
        self.conn
            .execute(
                r"
            INSERT OR REPLACE INTO content_vectors (hash, seq, pos, model, embedded_at, chunk_hash)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ",
                params![hash, seq as i64, pos as i64, model, embedded_at, chunk_hash],
            )
            .map_err(|e| format!("insert_embedding metadata: {e}"))?;

        let hash_seq = format!("{hash}_{seq}");
        let embedding_bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();

        self.conn
            .execute(
                "INSERT OR REPLACE INTO vectors_vec (hash_seq, embedding) VALUES (?1, ?2)",
                params![hash_seq, embedding_bytes],
            )
            .map_err(|e| format!("insert_embedding blob: {e}"))?;

        Ok(())
    }

    /// Check if a chunk needs re-embedding based on its content hash (#1107).
    ///
    /// Returns `true` if no embedding exists for this (hash, seq) pair, or if
    /// the stored chunk_hash differs from the new chunk_hash. Returns `false`
    /// if the chunk is unchanged and can be skipped.
    pub fn chunk_needs_embedding(
        &self,
        hash: &str,
        seq: usize,
        chunk_hash: &str,
    ) -> Result<bool, String> {
        // `optional()` turns "no such row" into None, so the closure itself
        // must read the column as Option<String>: a healed legacy row has a
        // NULL chunk_hash, and reading it as plain String would raise
        // InvalidColumnType(Null) instead of falling through to re-embed (#14).
        let stored_hash: Option<String> = self
            .conn
            .query_row(
                "SELECT chunk_hash FROM content_vectors WHERE hash = ?1 AND seq = ?2",
                params![hash, seq as i64],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|e| format!("chunk_needs_embedding: {e}"))?
            .flatten();

        match stored_hash {
            None => Ok(true),                         // No embedding exists yet
            Some(stored) => Ok(stored != chunk_hash), // Re-embed if hash changed
        }
    }

    /// Counts describing how much of the store is actually vectorised (#1067).
    ///
    /// Counts only, never document bodies: `/doctor` runs on a channel and must
    /// not pull memory content into a group chat, and the queries stay cheap on
    /// a large store.
    pub fn vector_stats(&self) -> Result<VectorStats, String> {
        let scalar = |sql: &str| -> Result<i64, String> {
            self.conn
                .query_row(sql, [], |r| r.get(0))
                .map_err(|e| format!("vector_stats: {e}"))
        };

        Ok(VectorStats {
            documents_active: scalar("SELECT COUNT(*) FROM documents WHERE active = 1")? as usize,
            // Same shape as `get_hashes_needing_embedding`, counted rather than
            // materialised: doctor wants the number, not 65 document bodies.
            documents_unembedded: scalar(
                r"
                SELECT COUNT(DISTINCT d.hash)
                FROM documents d
                LEFT JOIN content_vectors v ON d.hash = v.hash AND v.seq = 0
                WHERE d.active = 1 AND v.hash IS NULL
                ",
            )? as usize,
            vector_rows: scalar("SELECT COUNT(*) FROM content_vectors")? as usize,
            last_embedded_at: self
                .conn
                .query_row("SELECT MAX(embedded_at) FROM content_vectors", [], |r| {
                    r.get::<_, Option<String>>(0)
                })
                .map_err(|e| format!("vector_stats: {e}"))?,
        })
    }

    /// Active documents with no `seq = 0` vector row yet: (hash, path, body).
    pub fn get_hashes_needing_embedding(&self) -> Result<Vec<(String, String, String)>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r"
            SELECT DISTINCT d.hash, d.path, c.doc
            FROM documents d
            JOIN content c ON c.hash = d.hash
            LEFT JOIN content_vectors v ON d.hash = v.hash AND v.seq = 0
            WHERE d.active = 1 AND v.hash IS NULL
            ",
            )
            .map_err(|e| format!("get_hashes_needing_embedding: {e}"))?;

        let results = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(|e| format!("get_hashes_needing_embedding: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("get_hashes_needing_embedding: {e}"))?;

        Ok(results)
    }
}
