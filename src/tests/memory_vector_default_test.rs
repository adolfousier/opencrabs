//! #1798: `vector_enabled` must default to FALSE on every platform.
//!
//! Context: the native GGUF/llama.cpp engine aborts (0xC0000409) mid-index
//! on VPS and Windows hosts and used to take the daemon down with it,
//! because a fresh install had vectors ON by default and no chance to opt
//! out before the crash. Local embeddings still run well on Apple Silicon
//! and capable Linux desktops, so they are a deliberate opt-in now; these
//! guards pin that flip in both default paths (struct default and a
//! `[memory]` section that omits the key).

use crate::config::MemoryConfig;

#[test]
fn default_memory_config_has_vectors_off() {
    assert!(
        !MemoryConfig::default().vector_enabled,
        "#1798: vector_enabled must default off; opt in with vector_enabled = true"
    );
}

#[test]
fn omitted_key_deserializes_to_off() {
    let empty: MemoryConfig = toml::from_str("").expect("a [memory] section can be empty");
    assert!(
        !empty.vector_enabled,
        "serde default must match the struct default (off)"
    );

    let partial: MemoryConfig = toml::from_str("sweep_interval_secs = 120\n")
        .expect("a partial [memory] section must deserialize");
    assert!(
        !partial.vector_enabled,
        "presence of sibling keys must not resurrect the old true default"
    );
}

#[test]
fn explicit_true_still_opts_in() {
    let cfg: MemoryConfig =
        toml::from_str("vector_enabled = true\n").expect("opt-in line must parse");
    assert!(
        cfg.vector_enabled,
        "the opt-in must survive the default flip; machines that run the GGUF \
         engine well need the way back on"
    );
}
