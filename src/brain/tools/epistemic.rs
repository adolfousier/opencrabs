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

            let heading = normalize_heading(&section.heading);
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

// ---------------------------------------------------------------------------
// Archive pass (#1657 piece B): retire cold MEMORY.md sections into
// memory/archive/YYYY-MM.md instead of deleting them.
// ---------------------------------------------------------------------------

/// One section retired from MEMORY.md into the monthly archive file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedSection {
    /// The heading line as it appeared in MEMORY.md (e.g. `## Old Rules`).
    pub heading: String,
    /// Normalized title (no hashes) — the suffix of the belief key.
    pub title: String,
    /// The `MEMORY.md##<title>` belief key this section was tracked under.
    pub belief_key: String,
    /// The full section text (heading + body) as appended to the archive.
    pub rendered: String,
}

/// What `archive_cold_sections_at` did — the receipt a prune reports.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ArchiveReport {
    /// Sections archived, in file order.
    pub archived: Vec<ArchivedSection>,
    /// Monthly file the sections were appended to (`memory/archive/YYYY-MM.md`).
    pub archive_path: Option<PathBuf>,
    /// Backup snapshot of MEMORY.md taken before the survivor rewrite.
    pub backup: Option<PathBuf>,
    /// Belief keys removed from the epistemic store (one per section).
    pub removed_beliefs: Vec<String>,
    /// Nothing was cold enough to archive; no writes happened at all.
    pub skipped: bool,
}

/// Heading line → normalized title: `## Foo ##` → `Foo`.
///
/// Shared by backfill and the archive pass so a section's belief key and
/// its archive record can never disagree on what the section is called.
pub(crate) fn normalize_heading(raw: &str) -> String {
    raw.trim().trim_start_matches('#').trim().to_string()
}

/// Pure selection: which H2+ sections of `content` are cold enough to archive.
///
/// Cold means the section's belief exists AND matches the same yardstick as
/// `delete_cold_beliefs`: zero hits and unseen for `max_age_days` (measured
/// from `last_used`, falling back to `recorded_at`). Untracked sections are
/// never selected — no belief is not evidence of cold, it is evidence the
/// section predates tracking. The preamble and the H1 title are structurally
/// excluded by the same H2+ gate backfill applies, so a section that could
/// never be tracked can't be archived out from under the file either.
pub(crate) fn select_archive_candidates(
    store: &EpistemicStore,
    content: &str,
    max_age_days: i64,
    now: DateTime<Utc>,
) -> Vec<ArchivedSection> {
    let mut cold = Vec::new();
    for section in crate::brain::brain_sections::split_sections(content) {
        let trimmed = section.heading.trim();
        if trimmed.is_empty() || (trimmed.starts_with('#') && !trimmed.starts_with("##")) {
            continue;
        }
        let title = normalize_heading(&section.heading);
        let key = format!("MEMORY.md##{title}");
        let Some(belief) = store.beliefs.get(&key) else {
            continue;
        };
        let last_seen = belief.last_used.unwrap_or(belief.source.recorded_at);
        if belief.hits == 0 && (now - last_seen).num_days() >= max_age_days {
            cold.push(ArchivedSection {
                heading: trimmed.to_string(),
                title,
                belief_key: key,
                rendered: section.render(),
            });
        }
    }
    cold
}

/// Archive-pass core over explicit paths — the seam the tests drive.
///
/// Crash-safe ordering, in the only order that loses nothing:
/// 1. append the cold sections to the monthly archive file FIRST — a crash
///    after this step leaves duplicated content, which reconciliation
///    fixes, never lost content, which nothing fixes;
/// 2. snapshot MEMORY.md via `backup_before_write`;
/// 3. rewrite MEMORY.md with only the survivors (byte-exact: renders of
///    the untouched original sections, concatenated);
/// 4. drop the archived sections' beliefs from the hot store — if we die
///    before the caller saves, the beliefs merely outlive their sections
///    and the next pass re-collects them.
///
/// This deliberately shrinks a protected file: the shrink is the entire
/// point of the pass, and it is receipted (archive copy + backup + ledger)
/// rather than merely permitted. The write-tool `check_no_shrink` gate
/// governs tool-authored content; this maintenance pass carries its own
/// stronger guarantees.
pub(crate) fn archive_cold_sections_at(
    store: &mut EpistemicStore,
    memory_path: &std::path::Path,
    archive_dir: &std::path::Path,
    max_age_days: i64,
    now: DateTime<Utc>,
) -> Result<ArchiveReport, String> {
    let content = std::fs::read_to_string(memory_path)
        .map_err(|e| format!("archive: reading {}: {e}", memory_path.display()))?;

    let candidates = select_archive_candidates(store, &content, max_age_days, now);
    let mut report = ArchiveReport {
        skipped: candidates.is_empty(),
        ..Default::default()
    };
    if report.skipped {
        return Ok(report);
    }

    // 1. Archive copy first (crash = duplication, never loss).
    std::fs::create_dir_all(archive_dir)
        .map_err(|e| format!("archive: creating {}: {e}", archive_dir.display()))?;
    let archive_path = archive_dir.join(format!("{}.md", now.format("%Y-%m")));
    let payload: String = candidates.iter().map(|c| c.rendered.as_str()).collect();
    use std::io::Write as _;
    let mut archive_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&archive_path)
        .map_err(|e| format!("archive: opening {}: {e}", archive_path.display()))?;
    archive_file
        .write_all(payload.as_bytes())
        .map_err(|e| format!("archive: writing {}: {e}", archive_path.display()))?;
    drop(archive_file);

    // 2. Backup snapshot before the survivor rewrite.
    let backup = crate::brain::tools::brain_file_safety::backup_before_write(memory_path)
        .map_err(|e| format!("archive: backing up MEMORY.md: {e}"))?;

    // 3. Survivor rewrite — everything the pass did not select, byte-exact.
    let survivors: String = crate::brain::brain_sections::split_sections(&content)
        .into_iter()
        .filter(|s| !candidates.iter().any(|c| c.heading == s.heading.trim()))
        .map(|s| s.render())
        .collect();
    std::fs::write(memory_path, survivors)
        .map_err(|e| format!("archive: rewriting {}: {e}", memory_path.display()))?;

    // 4. Beliefs ride out with their sections; the archive file carries
    //    the content from here on (FTS keys it as archive/<YYYY-MM>.md).
    for c in &candidates {
        store.beliefs.remove(&c.belief_key);
    }

    report.removed_beliefs = candidates.iter().map(|c| c.belief_key.clone()).collect();
    report.archive_path = Some(archive_path);
    report.backup = backup;
    report.archived = candidates;
    Ok(report)
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

/// Runtime decay policy resolved from config (piece C, #1657).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DecaySettings {
    /// Master switch for decay: false skips the decay pass entirely.
    pub(crate) enabled: bool,
    /// Days before an unverified belief decays one confidence level.
    pub(crate) days: i64,
}

/// Map the `[epistemic]` section of ralph_loop.toml to decay settings.
///
/// `decay_interval_hours` divides down to whole days with a floor of 1:
/// a sub-day interval must not become "decay everything immediately"
/// (the silent-config-drop class of bug #1640 fixed).
pub(crate) fn decay_settings_from(cfg: &super::plan_tool::EpistemicConfig) -> DecaySettings {
    let days = (cfg.decay_interval_hours / 24).max(1) as i64;
    DecaySettings {
        enabled: cfg.decay_enabled,
        days,
    }
}

/// Resolve the active decay policy for this profile.
///
/// Reuses `ralph_loop_config` so per-project resolution (#947) and the
/// HotToml cache apply: `<profile_home>/ralph_loop.toml` first, then the
/// machine-wide `safety/ralph_loop.toml`. Absent or malformed config keeps
/// the pre-#1657 behavior: decay enabled, 30 days.
pub(crate) fn resolve_decay_settings() -> DecaySettings {
    let home = crate::config::profile::resolve_profile_home();
    match super::plan_tool::ralph_loop_config(&home) {
        Some(cfg) => decay_settings_from(&cfg.epistemic),
        None => DecaySettings {
            enabled: true,
            days: 30,
        },
    }
}

fn get_store() -> &'static std::sync::Mutex<EpistemicStore> {
    STORE.get_or_init(|| {
        let path = epistemic_store_path().expect("home dir must exist");
        let mut store = EpistemicStore::load(&path);

        // Session start maintenance: decay stale beliefs + backfill MEMORY.md (#1641).
        // Decay policy comes from [epistemic] in ralph_loop.toml (piece C, #1657);
        // absent config keeps the historical default: enabled, 30 days.
        let policy = resolve_decay_settings();
        let decayed = if policy.enabled {
            store.apply_decay(policy.days)
        } else {
            Vec::new()
        };
        if !decayed.is_empty() {
            tracing::info!("Epistemic session start: {} beliefs decayed", decayed.len());
        }

        // MEMORY.md lives at the profile root. The old walk from beliefs.toml
        // landed in brain/, so first-access backfill never found the file.
        let memory_path = crate::config::profile::resolve_profile_home().join("MEMORY.md");
        let mut added = 0;
        if let Ok(content) = std::fs::read_to_string(&memory_path) {
            added = store.backfill_from_content(&content);
            if added > 0 {
                tracing::info!(
                    "Epistemic session start: {} new beliefs backfilled from MEMORY.md",
                    added
                );
            }
        }

        // Save if anything changed: decayed beliefs or backfilled sections.
        if (!decayed.is_empty() || added > 0)
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

/// Archive cold MEMORY.md sections into `memory/archive/YYYY-MM.md` (#1657).
///
/// Wraps [`archive_cold_sections_at`] with the real paths plus the durable
/// side effects: epistemic store save, pruned-ledger `moved` records (the
/// same mechanism RSI header moves use, so template sync never resurrects
/// a retired section), and FTS reindex of both sides of the move so
/// `memory_search` sees it without waiting for a restart.
pub fn archive_cold_sections(max_age_days: i64) -> Result<ArchiveReport, String> {
    let home = crate::config::profile::resolve_profile_home();
    let memory_path = home.join("MEMORY.md");
    let archive_dir = home.join("memory").join("archive");

    // Core pass under the store lock; save only when something moved.
    let report = {
        let store = get_store();
        let mut guard = store.lock().expect("epistemic store lock poisoned");
        let r = archive_cold_sections_at(
            &mut guard,
            &memory_path,
            &archive_dir,
            max_age_days,
            Utc::now(),
        )?;
        if !r.skipped
            && let Some(path) = epistemic_store_path()
            && let Err(e) = guard.save(&path)
        {
            tracing::warn!("archive: saving epistemic store: {e}");
        }
        r
    };
    if report.skipped {
        return Ok(report);
    }

    // Ledger: record where each header moved. Home-relative dest so the
    // record reads as a path a human can open (`memory/archive/2026-09.md`).
    let archive_rel = report
        .archive_path
        .as_ref()
        .and_then(|p| p.strip_prefix(&home).ok())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "memory/archive".to_string());
    let mut pruned = crate::brain::rsi_pruned::PrunedState::load();
    pruned.record_moved(
        "MEMORY.md",
        report
            .archived
            .iter()
            .map(|c| crate::brain::rsi_pruned::MovedEntry {
                header: c.heading.clone(),
                dest: archive_rel.clone(),
            })
            .collect(),
    );
    if let Err(e) = pruned.save() {
        tracing::warn!("archive: saving pruned ledger: {e}");
    }

    // Reindex both sides of the move; without a tokio runtime the startup
    // reindex covers it on next boot rather than failing the pass.
    let reindex_targets: Vec<PathBuf> = report
        .archive_path
        .iter()
        .cloned()
        .chain(std::iter::once(memory_path))
        .collect();
    if let Ok(handle) = tokio::runtime::Handle::try_current()
        && let Ok(store) = crate::memory::get_store()
    {
        for path in reindex_targets {
            handle.spawn(async move {
                if let Err(e) = crate::memory::index_file_fts_only(store, &path).await {
                    tracing::warn!("archive: reindexing {}: {e}", path.display());
                }
            });
        }
    } else {
        tracing::debug!("archive: no tokio runtime, reindex deferred to startup");
    }

    Ok(report)
}
