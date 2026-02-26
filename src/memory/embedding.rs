//! Embedding — singleton engine, generate and store vector embeddings.

use once_cell::sync::OnceCell;
use qmd::{EmbeddingEngine, Store, pull_model};
use std::sync::Mutex;

static ENGINE: OnceCell<Mutex<EmbeddingEngine>> = OnceCell::new();

/// Disable llama.cpp's C-level logging globally.
///
/// Must be called once before creating any EmbeddingEngine.
/// Routes all llama.cpp log output through the tracing framework
/// with logging disabled — zero stderr pollution.
fn silence_llama_logs() {
    use llama_cpp_2::{LogOptions, send_logs_to_tracing};
    send_logs_to_tracing(LogOptions::default().with_logs_enabled(false));
}

/// Get (or create) the shared embedding engine.
///
/// Downloads the embeddinggemma-300M model (~300MB) on first call.
/// Returns Err if the download fails (e.g. no internet) or if the CPU lacks
/// AVX (required by llama.cpp GGUF inference) — callers fall back to FTS-only.
pub fn get_engine() -> Result<&'static Mutex<EmbeddingEngine>, String> {
    ENGINE.get_or_try_init(|| {
        check_cpu_features()?;
        silence_llama_logs();

        let pull = pull_model(qmd::llm::DEFAULT_EMBED_MODEL_URI, false)
            .map_err(|e| format!("Failed to pull embedding model: {e}"))?;

        let engine = EmbeddingEngine::new(&pull.path)
            .map_err(|e| format!("Failed to init embedding engine: {e}"))?;

        tracing::info!(
            "Embedding engine ready: {} ({:.1} MB)",
            pull.model,
            pull.size_bytes as f64 / 1_048_576.0
        );
        Ok(Mutex::new(engine))
    })
}

/// Verify the CPU supports the instruction sets required by llama.cpp.
/// Returns Err on x86 without AVX; passes through on ARM/other architectures.
fn check_cpu_features() -> Result<(), String> {
    #[cfg(target_arch = "x86_64")]
    {
        if !std::arch::is_x86_feature_detected!("avx") {
            return Err(
                "CPU lacks AVX — llama.cpp GGUF inference requires AVX (Sandy Bridge 2011+). \
                 Memory search will use FTS-only."
                    .to_string(),
            );
        }
    }
    Ok(())
}

/// Returns the engine if already initialized, without triggering a download.
pub fn engine_if_ready() -> Option<&'static Mutex<EmbeddingEngine>> {
    ENGINE.get()
}

/// Generate and store an embedding for content. No-ops if engine not yet initialized.
///
/// Lock ordering: engine first (embed), then store (insert). Never both at once.
pub fn embed_content(store: &Mutex<Store>, body: &str) {
    let engine_mutex = match engine_if_ready() {
        Some(e) => e,
        None => return,
    };

    let title = Store::extract_title(body);
    let hash = Store::hash_content(body);

    let emb = match engine_mutex.lock() {
        Ok(mut engine) => match engine.embed_document(body, Some(&title)) {
            Ok(emb) => emb,
            Err(e) => {
                tracing::debug!("Embedding failed: {e}");
                return;
            }
        },
        Err(_) => return,
    };

    // Store lock → insert → release
    let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    if let Ok(s) = store.lock()
        && let Err(e) = s.insert_embedding(&hash, 0, 0, &emb.embedding, &emb.model, &now)
    {
        tracing::debug!("Failed to store embedding: {e}");
    }
}

/// Backfill embeddings for all documents that don't have one yet.
///
/// Initializes the engine (downloading the model if needed) and batch-embeds
/// any documents missing embeddings. Lock ordering: store → release → engine → release → store.
pub(super) fn backfill_embeddings(store: &Mutex<Store>) {
    let engine_mutex = match get_engine() {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("Embedding engine unavailable, skipping backfill: {e}");
            return;
        }
    };

    // Store lock: get hashes needing embeddings → release
    let needing = match store.lock() {
        Ok(s) => s.get_hashes_needing_embedding().unwrap_or_default(),
        Err(_) => return,
    };

    if needing.is_empty() {
        return;
    }

    let count = needing.len();
    tracing::info!("Backfilling embeddings for {count} documents");

    // Process one document at a time, releasing the engine lock between each
    // so other callers (session_search, embed_content) aren't blocked for the
    // entire batch duration.
    let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    let mut stored = 0usize;

    for (i, (hash, _path, body)) in needing.iter().enumerate() {
        tracing::debug!("Embedding progress: {}/{}", i, count);

        let title = Store::extract_title(body);

        // Engine lock: embed single document → release
        let emb = {
            let mut engine = match engine_mutex.lock() {
                Ok(e) => e,
                Err(_) => return,
            };
            engine.embed_document(body, Some(&title)).ok()
        };

        // Store lock: insert embedding → release
        if let Some(emb) = emb
            && let Ok(s) = store.lock()
            && s.insert_embedding(hash, 0, 0, &emb.embedding, &emb.model, &now)
                .is_ok()
        {
            stored += 1;
        }
    }

    tracing::info!("Backfilled {stored}/{count} embeddings");
}
