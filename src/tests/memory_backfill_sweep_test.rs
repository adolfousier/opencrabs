//! The periodic embedding backfill decides correctly when to run (#1069).
//!
//! Embedding happened at startup and on write, and neither retries. A backfill
//! that failed for a transient reason left the vector table at zero until
//! someone restarted the process, with no error surfaced: search quietly
//! degraded to keyword-only FTS and kept answering.
//!
//! What is exercised here is the sweep's decision logic, not the embedding
//! call. That call is network-bound on the API path and loads a GGUF model on
//! the local one, so a test that drives it end to end would prove the endpoint
//! is up rather than that the sweep is correct. Stated rather than hidden.

use crate::config::{EmbeddingConfig, MemoryConfig};
use crate::memory::backfill_sweep::{interval_for, leave, try_enter};

fn cfg(vector_enabled: bool, interval: u64) -> MemoryConfig {
    MemoryConfig {
        vector_enabled,
        embedding: None,
        backfill_interval_secs: interval,
        ..Default::default()
    }
}

#[test]
fn the_default_interval_is_five_minutes() {
    // Chosen against the failure it fixes: a key corrected in config.toml
    // should take effect while the user is still looking at the terminal.
    // Vectors are off by default now (#1798), so the bare default carries no
    // sweep; the 300s figure belongs to an opted-in install. That the default
    // is off is pinned by memory_vector_default_test, and off-means-no-sweep
    // by disabled_vectors_disable_the_sweep_regardless_of_interval below.
    let opted_in = MemoryConfig {
        vector_enabled: true,
        ..Default::default()
    };
    assert_eq!(interval_for(&opted_in).map(|d| d.as_secs()), Some(300));
}

#[test]
fn an_explicit_interval_is_honoured() {
    assert_eq!(interval_for(&cfg(true, 60)).map(|d| d.as_secs()), Some(60));
    assert_eq!(
        interval_for(&cfg(true, 3600)).map(|d| d.as_secs()),
        Some(3600)
    );
}

#[test]
fn zero_disables_the_sweep() {
    assert_eq!(interval_for(&cfg(true, 0)), None);
}

#[test]
fn disabled_vectors_disable_the_sweep_regardless_of_interval() {
    // #1062 established that `vector_enabled = false` skips embedding work
    // entirely, local and API alike. A timer that wakes every five minutes to
    // decide it has nothing to do is still work the user asked not to happen.
    assert_eq!(interval_for(&cfg(false, 300)), None);
    assert_eq!(interval_for(&cfg(false, 60)), None);
}

#[test]
fn an_embedding_section_does_not_change_the_decision() {
    // The API/local branch is chosen inside the sweep, per tick. The interval
    // must not depend on it, or an install would silently lose its sweep by
    // switching backends.
    let mut with_api = cfg(true, 120);
    with_api.embedding = Some(EmbeddingConfig::default());
    assert_eq!(interval_for(&with_api).map(|d| d.as_secs()), Some(120));
}

#[test]
fn a_second_sweep_skips_while_the_first_holds_the_slot() {
    // Not politeness. `freshness.rs` documents that llama-cpp GGML can segfault
    // under contention, and a sweep landing on a still-running one is exactly a
    // second embedder entering alongside the first.
    assert!(try_enter(), "the slot must be free to begin with");
    assert!(!try_enter(), "a second entry must be refused, not queued");
    leave();
    assert!(try_enter(), "the slot must be reusable after release");
    leave();
}
