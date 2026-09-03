//! The set of real config sections, and what a write may address (#1199, #83).
//!
//! Reads and writes disagreed about what a section is. The read path resolves
//! shorthand (`stt` -> `providers`) against a registry; the write path
//! accepted any dotted string and created whatever tables it named. A caller
//! that passed `opencode` instead of `providers.opencode` got `Ok(())`, an
//! orphan `[opencode]` table at the end of `config.toml`, and a value serde
//! discards on every load. Reads then keep reporting the old value, which
//! looks exactly like a stale cache.
//!
//! #83 made the compiled struct the source of truth. A hand-maintained list
//! had drifted twice already: `CONFIG_SECTIONS` missed eight real sections
//! (daemon, a2a, image, cron, memory, brain, browser, doctor) and still
//! listed the migrated-away `[voice]`; `KNOWN_TOP_LEVEL_KEYS` missed
//! `doctor`. Both lists are gone. The write guard and the loader's typo
//! warning deserialize the document through `Config` with `serde_ignored` and
//! act on the ignored-key paths: the struct knows `[memory]`, so a write to
//! it passes; a path the struct would discard on load is refused outright,
//! keeping the #1199 silent-no-op hole closed at every depth, not just the
//! top level.
//!
//! The registry lives here rather than in the config tool because both sides
//! need it and `config` cannot depend on `brain::tools`.

use std::sync::OnceLock;

use crate::config::Config;

/// Top-level tables that actually exist in `config.toml`.
///
/// Kept for convenience and compatibility with tests; see [`known_sections`] for the
/// runtime schema-derived registry.
pub const CONFIG_SECTIONS: &[&str] = &[
    "agent",
    "a2a",
    "brain",
    "browser",
    "channels",
    "cron",
    "daemon",
    "database",
    "debug",
    "doctor",
    "image",
    "logging",
    "memory",
    "provider_registry",
    "providers",
    "tui",
];

/// Children whose parent section is not guessable from the name alone.
///
/// Used two ways: the read path accepts them as shorthand, and the write
/// guard turns them into the suggestion in its error, since a caller writing
/// `fallback` almost certainly means `providers.fallback`.
pub const SECTION_PARENTS: &[(&str, &str)] = &[
    ("telegram", "channels"),
    ("discord", "channels"),
    ("slack", "channels"),
    ("whatsapp", "channels"),
    ("trello", "channels"),
    ("stt", "providers"),
    ("tts", "providers"),
    ("fallback", "providers"),
    ("custom", "providers"),
];

/// The top-level sections `Config` deserializes, derived from the compiled
/// struct — never written by hand (#83).
///
/// Serialization is hole-free at the top level: every field of `Config` is a
/// plain `#[serde(default)] pub field: Type`, so `serde_json::to_value` shows
/// all of them (no `skip_serializing_if` on any top-level field). The read
/// side resolves shorthand against this; the WRITE guard skips the list
/// entirely and lets `serde_ignored` judge the candidate document, so this
/// cache is convenience for reads and suggestions, not the gate.
pub fn known_sections() -> &'static [String] {
    static KNOWN: OnceLock<Vec<String>> = OnceLock::new();
    KNOWN.get_or_init(|| {
        let mut names: Vec<String> = serde_json::to_value(Config::default())
            .expect("Config::default serializes")
            .as_object()
            .map(|obj| obj.keys().cloned().collect())
            .unwrap_or_default();
        names.sort();
        names
    })
}

/// Resolve what a READER asked for to a top-level section (#889).
///
/// Config is nested but the config tool only renders the first level, so the
/// paths people actually write were rejected: every recorded failure was
/// `providers.stt`, `stt` or `telegram`. Accepts an exact section, a dotted
/// path (`providers.stt` -> `providers`), or a known child
/// (`telegram` -> `channels`). `None` when nothing matches, so the caller
/// can refuse rather than guess.
pub fn resolve_section(requested: &str) -> Option<String> {
    let want = requested.trim().trim_matches('.').to_lowercase();
    if want.is_empty() {
        return None;
    }
    let head = want.split('.').next().unwrap_or(&want);
    // `gateway` is a serde alias of `a2a`; the struct field name is the
    // canonical spelling.
    if head == "gateway" {
        return Some("a2a".to_string());
    }
    if known_sections().iter().any(|s| s == head) {
        return Some(head.to_string());
    }
    SECTION_PARENTS
        .iter()
        .find(|(child, _)| *child == head)
        .map(|(_, parent)| (*parent).to_string())
}

/// Deserialize `content` through `Config`, collecting the dotted path of
/// every key the struct ignores (the `serde_ignored` pass — the compiled
/// struct IS the registry, so "ignored" means "the app discards this on
/// load"). `Err` when `content` does not deserialize at all.
pub fn ignored_key_paths(content: &str) -> Result<Vec<String>, String> {
    let de = toml::Deserializer::new(content);
    let mut ignored: Vec<String> = Vec::new();
    let res: Result<Config, _> = serde_ignored::deserialize(de, |path| {
        ignored.push(path.to_string());
    });
    res.map_err(|e| e.to_string())?;
    Ok(ignored)
}

/// Ignored TOP-LEVEL sections in `content` (single-segment ignored paths) —
/// what the loader's typo warning reports at load time.
pub fn unknown_top_level_sections(content: &str) -> Result<Vec<String>, String> {
    Ok(ignored_key_paths(content)?
        .into_iter()
        .filter(|p| !p.contains('.'))
        .collect())
}

/// Can a WRITE address `section`/`key` in the candidate document?
///
/// `section` is the dotted path as actually written into the document (the
/// table path `write_item` navigated), `candidate` the serialized document
/// AFTER the key was inserted — so the check reflects the locked live file,
/// with no TOCTOU window. The candidate is deserialized through `Config`
/// with `serde_ignored`; the write is valid if and only if the struct does
/// not ignore a key at the target path: no ignored path equals the target
/// or is a strict prefix of it (an unknown section is reported at its head,
/// an unknown field under a known section at the full path). Map-valued
/// sections (`providers.custom.*`, `channels.telegram.groups.*`) never
/// report ignored keys — map contents are user-chosen, not schema.
///
/// Returns `Ok(())` or a message naming the unknown path, the likely intent
/// (a `SECTION_PARENTS` child or the closest known section), and the fact
/// that the file was not changed.
pub fn write_guard(section: &str, key: &str, candidate: &str) -> Result<(), String> {
    reject_bad_shape(section)?;
    let section = section.trim().trim_matches('.');
    let target = format!("{section}.{key}");
    let ignored = ignored_key_paths(candidate)?;
    let blocked = ignored
        .iter()
        .any(|p| p == &target || target.starts_with(&format!("{p}.")));
    if !blocked {
        return Ok(());
    }

    let head = section.split('.').next().unwrap_or(section);

    // `voice` reads like a section (the derived view) so people try to write
    // it; steer them at the real tables instead of a generic rejection (#1385
    // — before this, the write passed the gate and serde discarded the table).
    if head == "voice" {
        return Err(
            "'voice' is a derived, read-only view over providers.stt/providers.tts — \
             it cannot be written. Set the underlying keys instead (e.g. \
             providers.stt.model, providers.tts.voice)."
                .to_string(),
        );
    }
    let suggestion = SECTION_PARENTS
        .iter()
        .find(|(child, _)| *child == head)
        .map(|(_, parent)| format!(" — did you mean '{parent}.{section}'?"))
        .or_else(|| suggest(head).map(|s| format!(" — did you mean '{s}'?")))
        .unwrap_or_default();
    Err(format!(
        "unknown config path '{target}': the Config struct has no such section or key{suggestion}. \
         Writing it would create a table/key serde ignores on load, so the value would silently \
         never apply (#1199). The file was NOT changed. Known sections: {}.",
        known_sections().join(", ")
    ))
}

/// Refuse section strings that would name junk tables in the document:
/// empty, padded, or double-dotted paths would create tables/key names the
/// struct ignores on load.
fn reject_bad_shape(section: &str) -> Result<(), String> {
    let trimmed = section.trim();
    if trimmed.is_empty() {
        return Err("config section is empty".to_string());
    }
    if trimmed != section {
        return Err(format!(
            "config section '{section}' has leading/trailing whitespace — it would name a table \
             the Config struct ignores on load. Use '{trimmed}'."
        ));
    }
    if section.split('.').any(|p| p.is_empty()) {
        return Err(format!(
            "config section '{section}' has an empty segment — it would name a table the Config \
             struct ignores on load. Use one dot between segments."
        ));
    }
    Ok(())
}

/// Nearest known section to `unknown` by edit distance, when close enough to
/// be a plausible typo (distance ≤ 2). `None` otherwise, so the caller falls
/// back to listing all known sections.
fn suggest(unknown: &str) -> Option<String> {
    let unknown = unknown.to_lowercase();
    known_sections()
        .iter()
        .map(|s| (edit_distance(&unknown, s), s.as_str()))
        .filter(|(d, _)| *d <= 2)
        .min_by_key(|(d, _)| *d)
        .map(|(_, s)| s.to_string())
}

/// Classic Levenshtein distance — small, dependency-free, only used for
/// typo suggestions.
fn edit_distance(a: &str, b: &str) -> usize {
    if a == b {
        return 0;
    }
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push(
                (prev[j + 1] + 1)
                    .min(cur[j] + 1)
                    .min(prev[j] + usize::from(ca != cb)),
            );
        }
        prev = cur;
    }
    prev[b.len()]
}
