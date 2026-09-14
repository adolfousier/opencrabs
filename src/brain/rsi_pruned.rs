//! Pruned Sections Tracker
//!
//! Tracks brain-file sections (## and ### headers) that the user has
//! intentionally deleted, so that `sync_templates()` does not re-add
//! them on the next upstream sync.
//!
//! State is persisted to `~/.opencrabs/rsi/pruned.toml`:
//! ```toml
//! schema_version = 2
//!
//! [SOUL.md]
//! pruned = ["## Old Section", "### Old Subsection"]
//! pruned_at = "2026-06-04T10:00:00Z"
//! moved = [
//!   ["## Old Home", "AGENTS.md"],
//!   ["### Subnote", "MEMORY.md"],
//! ]
//! ```
//!
//! Hook points:
//! - `write_opencrabs_file` tool: when cleanup_intent
//!   shrinks a protected brain file, diff old vs new to find removed
//!   sections and record them.
//! - `sync_templates()`: before appending a new upstream section,
//!   check if its header is in the pruned list and skip if so.
//!
//! Schema versioning: `schema_version` lets future format changes detect
//! and migrate older sidecars. v1 had no version field (treated as
//! implicit version 1). v2 adds `moved` per-file mappings so a header
//! that was relocated to a different brain file isn't re-added on the
//! source side by upstream sync.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Current on-disk schema version. v1 was unversioned (only `pruned` +
/// `pruned_at` per file). v2 adds `moved` (header → destination filename)
/// so syncs can skip headers the user explicitly relocated.
pub const SCHEMA_VERSION: u32 = 2;

/// One `moved` record: a header that the user relocated from its source
/// brain file to a different one. Stored as a struct (not a tuple) so
/// every read site uses named fields instead of `(h, _)` / `(_, dest)`
/// destructures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovedEntry {
    /// The `## Header` line that was moved from the source file.
    pub header: String,
    /// Destination filename the header was moved to (e.g. "AGENTS.md").
    pub dest: String,
}

/// In-memory representation of the pruned sidecar.
#[derive(Debug, Clone, PartialEq)]
pub struct PrunedState {
    /// Sidecar schema version. Always the loaded value; `save()` always
    /// writes `SCHEMA_VERSION` (i.e. older sidecars get upgraded in place
    /// the first time we write to them). Issue #164 edge-cases call this
    /// out explicitly so future format changes don't break older files.
    pub schema_version: u32,
    /// Map from filename (e.g. "SOUL.md") to list of pruned headers.
    pub pruned: HashMap<String, Vec<String>>,
    /// Map from filename to ISO-8601 timestamp of last prune event.
    pub pruned_at: HashMap<String, String>,
    /// Per-file list of `MovedEntry` records for headers the user moved
    /// to a different brain file. `sync_templates()` uses this to skip
    /// re-adding the header on the source side AND to warn when the
    /// destination doesn't exist (e.g. user later deleted it, the
    /// upstream renamed it, or the destination was never created).
    pub moved: HashMap<String, Vec<MovedEntry>>,
}

impl Default for PrunedState {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            pruned: HashMap::new(),
            pruned_at: HashMap::new(),
            moved: HashMap::new(),
        }
    }
}

impl PrunedState {
    /// Load state from `~/.opencrabs/rsi/pruned.toml`.
    /// Returns default (empty) state if file does not exist or fails to parse.
    pub fn load() -> Self {
        let path = Self::state_path();
        if !path.exists() {
            return Self::default();
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("pruned: failed to read pruned.toml: {e}");
                return Self::default();
            }
        };
        Self::parse(&content)
    }

    /// Parse TOML content into PrunedState.
    /// Simple hand-rolled parser to avoid pulling in a TOML crate just
    /// for this one file. Format is simple enough.
    ///
    /// v1 sidecars (no `schema_version` line) parse as version 1 and
    /// have no `moved` entries — `save()` will rewrite them at the
    /// current `SCHEMA_VERSION` on the next write.
    ///
    /// `pub(crate)` so the regression tests under `src/tests/` can
    /// exercise the parser directly without file I/O.
    pub(crate) fn parse(content: &str) -> Self {
        let mut state = Self {
            schema_version: 1,
            pruned: HashMap::new(),
            pruned_at: HashMap::new(),
            moved: HashMap::new(),
        };
        let mut current_file: Option<String> = None;
        // `moved` arrays can span multiple lines (one pair per row), so
        // we have to buffer until the closing `]`.
        let mut moved_buffer: Option<String> = None;

        for line in content.lines() {
            let raw = line;
            let trimmed = raw.trim();

            // If we're inside a multi-line moved array, append until we
            // hit the terminating `]`.
            if let Some(ref mut buf) = moved_buffer {
                buf.push_str(raw);
                buf.push('\n');
                if trimmed.ends_with(']') {
                    let buf_taken = std::mem::take(buf);
                    moved_buffer = None;
                    if let Some(ref file) = current_file {
                        let pairs = Self::parse_moved_array(&buf_taken);
                        if !pairs.is_empty() {
                            state.moved.insert(file.clone(), pairs);
                        }
                    }
                }
                continue;
            }

            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            // Top-level `schema_version = N`
            if current_file.is_none()
                && let Some((key, value)) = trimmed.split_once('=')
                && key.trim() == "schema_version"
                && let Ok(v) = value.trim().parse::<u32>()
            {
                state.schema_version = v;
                continue;
            }
            // Section header: [SOUL.md]
            if trimmed.starts_with('[') && trimmed.ends_with(']') && !trimmed.contains('=') {
                let name = trimmed[1..trimmed.len() - 1].trim().to_string();
                current_file = Some(name);
                continue;
            }
            if let Some(ref file) = current_file
                && let Some((key, value)) = trimmed.split_once('=')
            {
                let key = key.trim();
                let value = value.trim();
                if key == "pruned_at" {
                    let ts = value.trim_matches('"').to_string();
                    state.pruned_at.insert(file.clone(), ts);
                } else if key == "pruned" {
                    let headers = Self::parse_string_array(value);
                    if !headers.is_empty() {
                        state.pruned.insert(file.clone(), headers);
                    }
                } else if key == "moved" {
                    // Single-line case: closes on same line.
                    if value.starts_with('[') && value.ends_with(']') {
                        let pairs = Self::parse_moved_array(value);
                        if !pairs.is_empty() {
                            state.moved.insert(file.clone(), pairs);
                        }
                    } else if value.starts_with('[') {
                        // Multi-line case: start buffering.
                        moved_buffer = Some(format!("{value}\n"));
                    }
                }
            }
        }
        state
    }

    /// Parse a TOML array of `[header, destination]` pairs into a list
    /// of `MovedEntry`. Accepts both single-line `[["a", "b"], ["c", "d"]]`
    /// and multi-line forms. Each pair must be a two-element string array;
    /// malformed entries are silently skipped to keep older sidecars
    /// loading even when a hand-edited line is wrong.
    fn parse_moved_array(value: &str) -> Vec<MovedEntry> {
        let mut depth = 0i32;
        let mut current = String::new();
        let mut pairs_raw: Vec<String> = Vec::new();
        for ch in value.chars() {
            match ch {
                '[' => {
                    depth += 1;
                    if depth >= 2 {
                        current.push(ch);
                    }
                }
                ']' => {
                    if depth == 2 {
                        current.push(ch);
                        pairs_raw.push(std::mem::take(&mut current));
                    } else if depth >= 2 {
                        current.push(ch);
                    }
                    depth -= 1;
                }
                _ if depth >= 2 => current.push(ch),
                _ => {}
            }
        }
        pairs_raw
            .into_iter()
            .filter_map(|raw| {
                let strings = Self::parse_string_array(&raw);
                if strings.len() == 2 {
                    Some(MovedEntry {
                        header: strings[0].clone(),
                        dest: strings[1].clone(),
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    /// Parse a TOML array of strings like `["## Foo", "### Bar"]`.
    fn parse_string_array(value: &str) -> Vec<String> {
        let trimmed = value.trim();
        if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
            return Vec::new();
        }
        let inner = &trimmed[1..trimmed.len() - 1];
        let mut result = Vec::new();
        let mut current = String::new();
        let mut in_string = false;
        let mut escape = false;

        for ch in inner.chars() {
            if escape {
                current.push(ch);
                escape = false;
                continue;
            }
            if ch == '\\' {
                escape = true;
                continue;
            }
            if ch == '"' {
                if in_string {
                    result.push(current.clone());
                    current.clear();
                    in_string = false;
                } else {
                    in_string = true;
                }
                continue;
            }
            if in_string {
                current.push(ch);
            }
        }
        result
    }

    /// Save state to `~/.opencrabs/rsi/pruned.toml`.
    ///
    /// Always writes `schema_version = SCHEMA_VERSION` at the top.
    /// Older v1 sidecars get auto-upgraded the first time a write happens
    /// because `parse()` already loaded them into the new in-memory shape.
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::state_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut content = format!(
            "# Pruned brain-file sections.\n\
             # Sections listed here will NOT be re-added by sync_templates().\n\
             # Edit manually or use `opencrabs pruned clear` to reset.\n\n\
             schema_version = {SCHEMA_VERSION}\n\n",
        );

        // Stable order: union of all filenames that have any entry across
        // pruned / pruned_at / moved, sorted alphabetically. A file with
        // only a `moved` entry (no `pruned`) still gets its section.
        let mut files: HashSet<&String> = HashSet::new();
        files.extend(self.pruned.keys());
        files.extend(self.pruned_at.keys());
        files.extend(self.moved.keys());
        let mut files: Vec<&String> = files.into_iter().collect();
        files.sort();

        for file in files {
            let pruned_headers = self.pruned.get(file);
            let moved_pairs = self.moved.get(file);
            let has_any = pruned_headers.map(|h| !h.is_empty()).unwrap_or(false)
                || moved_pairs.map(|m| !m.is_empty()).unwrap_or(false);
            if !has_any {
                continue;
            }
            content.push_str(&format!("[{file}]\n"));
            if let Some(headers) = pruned_headers
                && !headers.is_empty()
            {
                content.push_str("pruned = [");
                let escaped: Vec<String> = headers
                    .iter()
                    .map(|h| format!("\"{}\"", h.replace('\\', "\\\\").replace('"', "\\\"")))
                    .collect();
                content.push_str(&escaped.join(", "));
                content.push_str("]\n");
            }
            if let Some(ts) = self.pruned_at.get(file) {
                content.push_str(&format!("pruned_at = \"{ts}\"\n"));
            }
            if let Some(pairs) = moved_pairs
                && !pairs.is_empty()
            {
                content.push_str("moved = [\n");
                for entry in pairs {
                    let h_esc = entry.header.replace('\\', "\\\\").replace('"', "\\\"");
                    let d_esc = entry.dest.replace('\\', "\\\\").replace('"', "\\\"");
                    content.push_str(&format!("  [\"{h_esc}\", \"{d_esc}\"],\n"));
                }
                content.push_str("]\n");
            }
            content.push('\n');
        }
        std::fs::write(&path, content)
    }

    fn state_path() -> PathBuf {
        crate::config::opencrabs_home().join("rsi/pruned.toml")
    }

    /// Check if a given header is pruned for a given file.
    pub fn is_pruned(&self, filename: &str, header: &str) -> bool {
        self.pruned
            .get(filename)
            .map(|headers| headers.iter().any(|h| h == header))
            .unwrap_or(false)
    }

    /// Get all pruned headers for a given file.
    pub fn pruned_headers(&self, filename: &str) -> HashSet<String> {
        self.pruned
            .get(filename)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect()
    }

    /// Record that a section was pruned from a file.
    /// Merges with existing pruned headers (no duplicates).
    pub fn record_pruned(&mut self, filename: &str, headers: Vec<String>) {
        if headers.is_empty() {
            return;
        }
        let entry = self.pruned.entry(filename.to_string()).or_default();
        for h in headers {
            if !entry.contains(&h) {
                entry.push(h);
            }
        }
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        self.pruned_at.insert(filename.to_string(), now);
    }

    /// Clear all pruned entries for a file (or all files if filename is None).
    /// Also clears `moved` entries for the same scope so a single
    /// "forget about this file" action resets both lists in one call.
    pub fn clear(&mut self, filename: Option<&str>) {
        if let Some(f) = filename {
            self.pruned.remove(f);
            self.pruned_at.remove(f);
            self.moved.remove(f);
        } else {
            self.pruned.clear();
            self.pruned_at.clear();
            self.moved.clear();
        }
    }

    /// Record that the user moved headers from `source_file` to one or
    /// more destinations. `sync_templates()` uses this on the source
    /// side to skip re-adding each header, and at warn time to flag
    /// missing destinations (e.g. the dest file was later deleted).
    /// No-op when `entries` is empty.
    pub fn record_moved(&mut self, source_file: &str, entries: Vec<MovedEntry>) {
        if entries.is_empty() {
            return;
        }
        let bucket = self.moved.entry(source_file.to_string()).or_default();
        for new_entry in entries {
            if !bucket.iter().any(|e| e.header == new_entry.header) {
                bucket.push(new_entry);
            }
        }
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        self.pruned_at.insert(source_file.to_string(), now);
    }

    /// Look up the destination file for a moved header. Returns `None`
    /// when the header isn't in the moved list for this source file.
    pub fn moved_destination(&self, source_file: &str, header: &str) -> Option<&str> {
        self.moved
            .get(source_file)
            .and_then(|entries| entries.iter().find(|e| e.header == header))
            .map(|e| e.dest.as_str())
    }
}

/// Diff two versions of a brain file and return the section headers
/// (## and ### lines) that were present in `old` but not in `new`.
pub fn detect_removed_sections(old: &str, new: &str) -> Vec<String> {
    let old_headers: HashSet<String> = crate::brain::rsi_sync::extract_section_headers(old)
        .into_iter()
        .collect();
    let new_headers: HashSet<String> = crate::brain::rsi_sync::extract_section_headers(new)
        .into_iter()
        .collect();
    let mut removed: Vec<String> = old_headers.difference(&new_headers).cloned().collect();
    removed.sort();
    removed
}

/// Filter out pruned sections from a string of new sections to append.
/// Returns the filtered string with pruned section blocks removed.
///
/// A header is filtered out if EITHER it appears in the file's `pruned`
/// list OR it appears as the source of a `moved` entry. Both mean
/// "the user explicitly decided this header doesn't belong here" — the
/// only difference is whether they relocated it elsewhere.
pub fn filter_pruned_sections(new_sections: &str, state: &PrunedState, filename: &str) -> String {
    let pruned = state.pruned_headers(filename);
    let moved_sources: HashSet<String> = state
        .moved
        .get(filename)
        .map(|entries| entries.iter().map(|e| e.header.clone()).collect())
        .unwrap_or_default();
    if pruned.is_empty() && moved_sources.is_empty() {
        return new_sections.to_string();
    }

    let mut blocks: Vec<(String, Vec<String>)> = Vec::new();
    let mut current_header = String::new();
    let mut current_content: Vec<String> = Vec::new();

    for line in new_sections.lines() {
        if line.starts_with("## ") || line.starts_with("### ") {
            if !current_header.is_empty() {
                blocks.push((current_header.clone(), current_content.clone()));
            }
            current_header = line.to_string();
            current_content = vec![line.to_string()];
        } else if !current_header.is_empty() {
            current_content.push(line.to_string());
        }
    }
    if !current_header.is_empty() {
        blocks.push((current_header, current_content));
    }

    let kept: Vec<String> = blocks
        .into_iter()
        .filter_map(|(header, content)| {
            if pruned.contains(&header) || moved_sources.contains(&header) {
                None
            } else {
                Some(content.join("\n"))
            }
        })
        .collect();

    if kept.is_empty() {
        String::new()
    } else {
        format!("\n{}\n", kept.join("\n\n"))
    }
}

// Tests live under `src/tests/rsi_pruned_test.rs` per project policy
// (no inline `#[cfg(test)] mod tests` blocks).
//
// `PrunedState::parse` is `pub(crate)` so the test file can reach the
// parser directly without going through file I/O.
