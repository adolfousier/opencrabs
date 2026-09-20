//! Epistemic Engine — Belief Tracking with Confidence Levels
//!
//! Tracks beliefs (facts, decisions, context) with confidence levels and
//! source attribution. Implements decay logic and contradiction detection.
//!
//! Design:
//! - Confidence levels: verified, inferred, uncertain, contradicted
//! - Source attribution: every belief tagged with origin
//! - Decay: unverified beliefs lose confidence after 30 days
//! - Contradiction detection: new fact conflicts existing belief → flagged
//! - Storage: ~/.opencrabs/brain/epistemic/beliefs.toml
//!
//! Config: ralph_loop.toml [epistemic] section (already exists)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Confidence levels for beliefs, ordered from most to least certain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// Conflicts with another belief — needs resolution (lowest confidence)
    Contradicted,
    /// Not yet verified, assumed true
    Uncertain,
    /// Derived from other beliefs or logical inference
    Inferred,
    /// Confirmed by user or system verification (highest confidence)
    Verified,
}

impl Confidence {
    /// Decay confidence by one level. Verified beliefs don't decay.
    pub fn decay(self) -> Self {
        match self {
            Confidence::Verified => Confidence::Verified,
            Confidence::Inferred => Confidence::Uncertain,
            Confidence::Uncertain => Confidence::Contradicted,
            Confidence::Contradicted => Confidence::Contradicted,
        }
    }

    /// Human-readable label
    pub fn label(&self) -> &'static str {
        match self {
            Confidence::Verified => "verified",
            Confidence::Inferred => "inferred",
            Confidence::Uncertain => "uncertain",
            Confidence::Contradicted => "contradicted",
        }
    }
}

/// Source attribution for a belief.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    /// Who/what provided this belief (e.g. "user:adolfo", "inference", "session:abc123")
    pub origin: String,
    /// When the belief was first recorded
    pub recorded_at: DateTime<Utc>,
    /// When the belief was last verified
    pub last_verified: DateTime<Utc>,
}

/// A single belief with confidence, source tracking, and usage metrics.
///
/// `hits` and `last_used` track how often and how recently a belief was
/// accessed. These drive the cold-facts deletion pass (#1641): beliefs
/// with 0 hits older than 90 days are candidates for removal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Belief {
    /// Unique key for this belief (e.g. "memory:truelens:staging_ip")
    pub key: String,
    /// The belief value (e.g. "159.65.49.225")
    pub value: String,
    /// Current confidence level
    pub confidence: Confidence,
    /// Source attribution
    pub source: Source,
    /// Optional notes or context
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Number of times this belief was accessed (recall injection or explicit get).
    #[serde(default)]
    pub hits: u64,
    /// Last time this belief was accessed. `None` means never used since
    /// tracking was added; decay and cold-facts logic falls back to
    /// `source.recorded_at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<DateTime<Utc>>,
}

/// The epistemic store — all tracked beliefs.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct EpistemicStore {
    /// Map of belief key → belief
    #[serde(default)]
    pub beliefs: HashMap<String, Belief>,
    /// Schema version for future migrations
    #[serde(default = "default_version")]
    pub version: u32,
}

fn default_version() -> u32 {
    1
}

impl EpistemicStore {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self {
            beliefs: HashMap::new(),
            version: 1,
        }
    }

    /// Add or update a belief. If the key exists and the value differs,
    /// the old belief is marked as contradicted.
    pub fn add_belief(
        &mut self,
        key: &str,
        value: &str,
        confidence: Confidence,
        origin: &str,
    ) -> ContradictionResult {
        let now = Utc::now();

        // Check for contradiction with existing belief.
        // Clone first to avoid borrow conflict (immutable get + mutable insert).
        let existing = self.beliefs.get(key).cloned();
        if let Some(existing) = existing
            && existing.value != value
            && existing.confidence != Confidence::Contradicted
        {
            // Extract old_value BEFORE mutable borrow
            let old_value = existing.value.clone();

            // Archive the superseded belief under its OWN namespace (#1083).
            // The archive key PREFIXES rather than extends the original: a
            // `{key}:contradicted:{ts}` suffix still starts with the original
            // key, so every superseded copy kept matching prefix queries
            // forever and piled up in whatever surface reads them. The `key`
            // field is set to the archive key too, so field and map key agree.
            let archive_key = format!("contradicted:{}:{}", now.timestamp(), key);
            let mut contradicted = existing.clone();
            contradicted.key = archive_key.clone();
            contradicted.confidence = Confidence::Contradicted;
            contradicted.notes = Some(format!(
                "Contradicted by new value '{}' from {} at {}",
                value,
                origin,
                now.format("%Y-%m-%d %H:%M:%S UTC")
            ));
            self.beliefs.insert(archive_key, contradicted);

            // Insert new belief
            let belief = Belief {
                key: key.to_string(),
                value: value.to_string(),
                confidence,
                source: Source {
                    origin: origin.to_string(),
                    recorded_at: now,
                    last_verified: now,
                },
                notes: None,
                hits: 0,
                last_used: None,
            };
            self.beliefs.insert(key.to_string(), belief);

            return ContradictionResult::Contradicted {
                old_value,
                new_value: value.to_string(),
            };
        }

        // No contradiction — insert or update
        let belief = Belief {
            key: key.to_string(),
            value: value.to_string(),
            confidence,
            source: Source {
                origin: origin.to_string(),
                recorded_at: now,
                last_verified: now,
            },
            notes: None,
            hits: 0,
            last_used: None,
        };
        self.beliefs.insert(key.to_string(), belief);

        ContradictionResult::NoContradiction
    }

    /// Get a belief by key.
    pub fn get_belief(&self, key: &str) -> Option<&Belief> {
        self.beliefs.get(key)
    }

    /// Re-verify a belief (updates last_verified timestamp).
    pub fn verify_belief(&mut self, key: &str) -> bool {
        if let Some(belief) = self.beliefs.get_mut(key) {
            belief.source.last_verified = Utc::now();
            belief.confidence = Confidence::Verified;
            true
        } else {
            false
        }
    }

    /// Record a usage hit on a belief: increment `hits` and set `last_used`
    /// to now. Returns true if the belief existed, false otherwise.
    ///
    /// Called from `memory_recall.rs` when a belief's section is injected
    /// into context (#1641).
    pub fn touch_belief(&mut self, key: &str) -> bool {
        if let Some(belief) = self.beliefs.get_mut(key) {
            belief.hits += 1;
            belief.last_used = Some(Utc::now());
            true
        } else {
            false
        }
    }

    /// Apply decay logic: beliefs not verified within `decay_days` drop
    /// one confidence level. Verified beliefs are immune.
    pub fn apply_decay(&mut self, decay_days: i64) -> Vec<String> {
        let now = Utc::now();
        let mut decayed = Vec::new();

        for belief in self.beliefs.values_mut() {
            if belief.confidence == Confidence::Verified {
                continue; // Verified beliefs don't decay
            }

            // Use last_used as the clock (fall back to recorded_at for
            // beliefs never touched since tracking was added). This is the
            // right clock: a belief used recently shouldn't decay even if
            // it was never explicitly re-verified (#1641).
            let last_seen = belief.last_used.unwrap_or(belief.source.recorded_at);
            let age_days = (now - last_seen).num_days();
            if age_days >= decay_days {
                let old = belief.confidence;
                belief.confidence = belief.confidence.decay();
                if belief.confidence != old {
                    decayed.push(format!(
                        "{}: {} → {} ({} days since last use)",
                        belief.key,
                        old.label(),
                        belief.confidence.label(),
                        age_days
                    ));
                }
            }
        }

        decayed
    }

    /// List beliefs filtered by confidence level.
    pub fn list_by_confidence(&self, confidence: Confidence) -> Vec<&Belief> {
        self.beliefs
            .values()
            .filter(|b| b.confidence == confidence)
            .collect()
    }

    /// List all contradicted beliefs (for review).
    pub fn list_contradictions(&self) -> Vec<&Belief> {
        self.list_by_confidence(Confidence::Contradicted)
    }

    /// List beliefs whose key starts with `prefix`.
    pub fn list_by_key_prefix(&self, prefix: &str) -> Vec<&Belief> {
        self.beliefs
            .values()
            .filter(|b| b.key.starts_with(prefix))
            .collect()
    }

    /// Save the store to disk.
    pub fn save(&self, path: &PathBuf) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, content)
    }

    /// Load the store from disk. Returns empty store if file missing.
    pub fn load(path: &PathBuf) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => match toml::from_str(&content) {
                Ok(store) => store,
                Err(e) => {
                    tracing::warn!("Epistemic store parse error: {}", e);
                    Self::new()
                }
            },
            Err(_) => Self::new(),
        }
    }

    /// Index MEMORY.md sections into beliefs from raw content (#1641).
    ///
    /// Each section becomes a belief keyed on its heading (e.g.
    /// `"MEMORY.md##Rules"`). Only sections with substantial body content
    /// (>20 chars) are indexed; stub headings and the preamble (no heading)
    /// are skipped. Existing beliefs are never overwritten, so verified
    /// beliefs survive a re-backfill.
    ///
    /// Returns the number of new beliefs added.
    pub fn backfill_from_content(&mut self, content: &str) -> usize {
        let sections = crate::brain::brain_sections::split_sections(content);
        let now = Utc::now();
        let mut added = 0usize;

        for section in &sections {
            let trimmed_heading = section.heading.trim();
            // Skip the preamble (no heading) and H1 document titles —
            // only H2+ sections carry operational content worth indexing.
            if trimmed_heading.is_empty()
                || (trimmed_heading.starts_with('#') && !trimmed_heading.starts_with("##"))
            {
                continue;
            }
            // Skip stub sections with negligible body content.
            if section.body.trim().len() < 20 {
                continue;
            }

            let heading = section.heading.trim().trim_start_matches('#').trim();
            let key = format!("MEMORY.md##{}", heading);

            // Don't overwrite existing beliefs — verified beliefs survive re-backfill.
            if self.beliefs.contains_key(&key) {
                continue;
            }

            let belief = Belief {
                key: key.clone(),
                value: section.body.trim().chars().take(200).collect(),
                confidence: Confidence::Inferred,
                source: Source {
                    origin: format!("MEMORY.md backfill ({})", now.format("%Y-%m-%d")),
                    recorded_at: now,
                    last_verified: now,
                },
                notes: None,
                hits: 0,
                last_used: None,
            };
            self.beliefs.insert(key, belief);
            added += 1;
        }

        added
    }

    /// Delete beliefs with 0 hits older than `max_age_days` (#1641).
    ///
    /// Cold facts are beliefs nobody ever recalled and that have been
    /// sitting untouched for 90+ days. Deleting them keeps the belief
    /// store from accumulating noise. Returns the keys of deleted beliefs.
    pub fn delete_cold_beliefs(&mut self, max_age_days: i64) -> Vec<String> {
        let now = Utc::now();
        let cold_keys: Vec<String> = self
            .beliefs
            .iter()
            .filter(|(_, b)| {
                b.hits == 0 && {
                    let last_seen = b.last_used.unwrap_or(b.source.recorded_at);
                    (now - last_seen).num_days() >= max_age_days
                }
            })
            .map(|(k, _)| k.clone())
            .collect();

        for key in &cold_keys {
            self.beliefs.remove(key);
        }

        cold_keys
    }

    /// List beliefs that WOULD be deleted by `delete_cold_beliefs` (dry-run).
    pub fn list_cold_beliefs(&self, max_age_days: i64) -> Vec<&Belief> {
        let now = Utc::now();
        self.beliefs
            .values()
            .filter(|b| {
                b.hits == 0 && {
                    let last_seen = b.last_used.unwrap_or(b.source.recorded_at);
                    (now - last_seen).num_days() >= max_age_days
                }
            })
            .collect()
    }
}

/// Result of adding a belief — indicates if a contradiction was detected.
#[derive(Debug, Clone, PartialEq)]
pub enum ContradictionResult {
    /// No contradiction — belief added/updated normally
    NoContradiction,
    /// Contradiction detected — old belief marked as contradicted
    Contradicted {
        old_value: String,
        new_value: String,
    },
}

/// Get the epistemic store path.
fn epistemic_store_path() -> Option<PathBuf> {
    let home = crate::config::profile::resolve_profile_home();
    Some(home.join("brain/epistemic/beliefs.toml"))
}

/// Global epistemic store (cached for session lifetime).
static STORE: OnceLock<std::sync::Mutex<EpistemicStore>> = OnceLock::new();

fn get_store() -> &'static std::sync::Mutex<EpistemicStore> {
    STORE.get_or_init(|| {
        let path = epistemic_store_path().expect("home dir must exist");
        let mut store = EpistemicStore::load(&path);

        // Session start maintenance: decay stale beliefs + backfill MEMORY.md (#1641).
        let decayed = store.apply_decay(30);
        if !decayed.is_empty() {
            tracing::info!("Epistemic session start: {} beliefs decayed", decayed.len());
        }

        let memory_path = path
            .parent()
            .unwrap_or(&path)
            .parent()
            .unwrap_or(&path)
            .join("MEMORY.md");
        if let Ok(content) = std::fs::read_to_string(&memory_path) {
            let added = store.backfill_from_content(&content);
            if added > 0 {
                tracing::info!(
                    "Epistemic session start: {} new beliefs backfilled from MEMORY.md",
                    added
                );
            }
        }

        // Save if anything changed
        if !decayed.is_empty()
            && let Err(e) = store.save(&path)
        {
            tracing::warn!("Failed to save epistemic store after session start: {}", e);
        }

        std::sync::Mutex::new(store)
    })
}

/// Add a belief to the global store. Returns contradiction result.
pub fn add_belief(
    key: &str,
    value: &str,
    confidence: Confidence,
    origin: &str,
) -> ContradictionResult {
    let store = get_store();
    let mut guard = store.lock().expect("epistemic store lock poisoned");
    let result = guard.add_belief(key, value, confidence, origin);

    // Auto-save after modification
    if let Some(path) = epistemic_store_path()
        && let Err(e) = guard.save(&path)
    {
        tracing::warn!("Failed to save epistemic store: {}", e);
    }

    result
}

/// Get a belief from the global store.
pub fn get_belief(key: &str) -> Option<Belief> {
    let store = get_store();
    let guard = store.lock().expect("epistemic store lock poisoned");
    guard.get_belief(key).cloned()
}

/// Verify a belief in the global store.
pub fn verify_belief(key: &str) -> bool {
    let store = get_store();
    let mut guard = store.lock().expect("epistemic store lock poisoned");
    let result = guard.verify_belief(key);

    if result
        && let Some(path) = epistemic_store_path()
        && let Err(e) = guard.save(&path)
    {
        tracing::warn!("Failed to save epistemic store: {}", e);
    }

    result
}

/// Touch a belief in the global store (increment hits, update last_used).
pub fn touch_belief(key: &str) -> bool {
    let store = get_store();
    let mut guard = store.lock().expect("epistemic store lock poisoned");
    let result = guard.touch_belief(key);

    if result
        && let Some(path) = epistemic_store_path()
        && let Err(e) = guard.save(&path)
    {
        tracing::warn!("Failed to save epistemic store: {}", e);
    }

    result
}

/// Apply decay to the global store. Returns list of decayed beliefs.
pub fn apply_decay(decay_days: i64) -> Vec<String> {
    let store = get_store();
    let mut guard = store.lock().expect("epistemic store lock poisoned");
    let decayed = guard.apply_decay(decay_days);

    if !decayed.is_empty()
        && let Some(path) = epistemic_store_path()
        && let Err(e) = guard.save(&path)
    {
        tracing::warn!("Failed to save epistemic store: {}", e);
    }

    decayed
}

/// List all contradicted beliefs in the global store.
pub fn list_contradictions() -> Vec<Belief> {
    let store = get_store();
    let guard = store.lock().expect("epistemic store lock poisoned");
    guard.list_contradictions().into_iter().cloned().collect()
}

/// List beliefs whose key starts with `prefix` from the global store.
pub fn list_by_prefix(prefix: &str) -> Vec<Belief> {
    let store = get_store();
    let guard = store.lock().expect("epistemic store lock poisoned");
    guard
        .list_by_key_prefix(prefix)
        .into_iter()
        .cloned()
        .collect()
}

/// Index existing MEMORY.md sections into beliefs (#1641).
///
/// Reads MEMORY.md from the profile home and delegates to
/// [`EpistemicStore::backfill_from_content`]. Returns the number of new
/// beliefs added.
pub fn backfill_beliefs() -> usize {
    let home = crate::config::profile::resolve_profile_home();
    let memory_path = home.join("MEMORY.md");
    let content = match std::fs::read_to_string(&memory_path) {
        Ok(c) => c,
        Err(_) => return 0,
    };

    let store = get_store();
    let mut guard = store.lock().expect("epistemic store lock poisoned");
    let added = guard.backfill_from_content(&content);

    if added > 0
        && let Some(path) = epistemic_store_path()
        && let Err(e) = guard.save(&path)
    {
        tracing::warn!("Failed to save epistemic store after backfill: {}", e);
    }

    added
}

/// Delete cold beliefs (0 hits, 90+ days) from the global store (#1641).
/// Returns the keys of deleted beliefs.
pub fn delete_cold_beliefs(max_age_days: i64) -> Vec<String> {
    let store = get_store();
    let mut guard = store.lock().expect("epistemic store lock poisoned");
    let deleted = guard.delete_cold_beliefs(max_age_days);

    if !deleted.is_empty()
        && let Some(path) = epistemic_store_path()
        && let Err(e) = guard.save(&path)
    {
        tracing::warn!("Failed to save epistemic store after cold delete: {}", e);
    }

    deleted
}

/// List cold beliefs (dry-run for `/memory prune`) from the global store.
pub fn list_cold_beliefs(max_age_days: i64) -> Vec<Belief> {
    let store = get_store();
    let guard = store.lock().expect("epistemic store lock poisoned");
    guard
        .list_cold_beliefs(max_age_days)
        .into_iter()
        .cloned()
        .collect()
}

/// Run epistemic maintenance at session start: decay + backfill (#1641).
///
/// Called once when the store is first loaded. Applies 30-day decay to
/// stale beliefs, indexes any new MEMORY.md sections, and logs results.
pub fn session_start_maintenance() {
    // Apply decay (30-day threshold)
    let decayed = apply_decay(30);
    if !decayed.is_empty() {
        tracing::info!("Epistemic session start: {} beliefs decayed", decayed.len());
    }

    // Backfill any new MEMORY.md sections
    let added = backfill_beliefs();
    if added > 0 {
        tracing::info!(
            "Epistemic session start: {} new beliefs backfilled from MEMORY.md",
            added
        );
    }
}
