//! Skill loader — embedded built-in skills with user-directory overlay.
//!
//! Skills are workflow templates: a `name`, a `description` (read by the
//! LLM to decide when to invoke), and a `body` (the actual instructions).
//! Format follows the de-facto `SKILL.md` convention used by Claude Code,
//! Anthropic managed agents, and OpenClaw — YAML frontmatter at the top
//! of the file, followed by the prompt body.
//!
//! Layout:
//!
//! ```text
//! ~/.opencrabs/skills/
//! └── <skill-name>/
//!     └── SKILL.md          ← user-owned, overrides any built-in of the same name
//! ```
//!
//! The repo ships a curated set of built-ins under
//! `src/docs/reference/templates/skills/<name>/SKILL.md`, embedded at
//! compile time via `include_str!`. The user directory at
//! `~/.opencrabs/skills/` is purely user-owned (per `TOOLS.md`); writes
//! never come from the binary.
//!
//! ## Resolution order
//!
//! 1. `~/.opencrabs/skills/<name>/SKILL.md` — user override
//! 2. embedded built-in
//!
//! A user file with a malformed frontmatter falls back to the built-in
//! (with a warning), so a broken local edit cannot brick the skill.
//!
//! ## Frontmatter
//!
//! ```markdown
//! ---
//! name: security-audit
//! description: Comprehensive security and CVE audit. Triggers on ...
//! ---
//!
//! Body of the skill...
//! ```
//!
//! Only `name` and `description` are recognised today. Other keys are
//! preserved for forward-compat but ignored.

use std::path::PathBuf;
use std::sync::OnceLock;

/// Compile-time table of built-in skills shipped with the binary.
///
/// To add a new built-in, drop a `SKILL.md` under
/// `src/docs/reference/templates/skills/<name>/` and add a line here.
const BUILTIN_SKILLS: &[(&str, &str)] = &[
    (
        "cost-estimate",
        include_str!("../docs/reference/templates/skills/cost-estimate/SKILL.md"),
    ),
    (
        "security-audit",
        include_str!("../docs/reference/templates/skills/security-audit/SKILL.md"),
    ),
    (
        "repo-audit",
        include_str!("../docs/reference/templates/skills/repo-audit/SKILL.md"),
    ),
    (
        "browser-cdp",
        include_str!("../docs/reference/templates/skills/browser-cdp/SKILL.md"),
    ),
    (
        "a2a-gateway",
        include_str!("../docs/reference/templates/skills/a2a-gateway/SKILL.md"),
    ),
    (
        "dynamic-tools",
        include_str!("../docs/reference/templates/skills/dynamic-tools/SKILL.md"),
    ),
    (
        "github",
        include_str!("../docs/reference/templates/skills/github/SKILL.md"),
    ),
    (
        "multi-agent",
        include_str!("../docs/reference/templates/skills/multi-agent/SKILL.md"),
    ),
];

/// The built-in table, exposed for tests (#990). Asserting through
/// `load_all_skills` cannot do this job: it merges built-ins with the user's
/// own `~/.opencrabs/skills/`, so a missing built-in is masked on any machine
/// that happens to carry a user copy of the same name.
pub const BUILTIN_SKILLS_FOR_TEST: &[(&str, &str)] = BUILTIN_SKILLS;

/// Where this skill came from. Used by the TUI to badge built-ins
/// differently from user-installed ones in the autocomplete dropdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillSource {
    Builtin,
    User,
}

#[derive(Debug, Clone)]
pub struct Skill {
    /// Slug used to invoke the skill (`/security-audit` → `"security-audit"`).
    pub name: String,
    /// Precomputed `/<name>` form, for autocomplete display and sort
    /// comparisons against built-in / user-command names that already
    /// carry the leading slash.
    pub slash_name: String,
    /// One-line summary the LLM reads to decide when to invoke.
    pub description: String,
    /// Prompt body (everything after the closing `---`, trimmed).
    pub body: String,
    /// Cursor-style glob paths. When non-empty, the skill gate rejects
    /// tool calls touching a matching path until the skill body has been
    /// loaded (seen) in the current session context. Opt-in: no `globs`
    /// key → empty vec → invisible to the gate.
    pub globs: Vec<String>,
    /// `review_gate: true` in frontmatter: slash invocation of this skill
    /// is the user reaching for the brake on purpose. The agent must
    /// present the skill's output and wait for explicit user approval
    /// before any side effects, tool auto-approve notwithstanding.
    /// Natural-language requests that never touch the slash keep the
    /// usual flow — the gate lives on the slash, not the skill's topic.
    pub review_gate: bool,
    pub source: SkillSource,
}

/// Hard reminder prepended to a skill's body when it declares
/// `review_gate: true`. The gate is a property of the skill as invoked
/// via its slash: output first, side effects only after the user's
/// explicit approval — even under tool auto-approve (yolo).
pub const REVIEW_GATE_REMINDER: &str = "[SKILL REVIEW GATE] This skill declares `review_gate: true`. \
     Present its output (draft, plan, summary) to the user and WAIT for their explicit \
     approval before any side effects — sending, publishing, pushing, deploying, or writing \
     outside the workspace — even if tool auto-approve is on. The user typed the slash \
     because they want to review first; if they had wanted end-to-end execution they would \
     have asked in plain words.";

impl Skill {
    /// Parse a `SKILL.md` blob into a `Skill`. Returns `Err` if the
    /// frontmatter is missing required fields.
    pub fn parse(name: &str, raw: &str, source: SkillSource) -> Result<Self, String> {
        // Normalise CRLF → LF and strip BOM so the line-walking logic
        // below has a single shape to handle.
        let raw = raw.strip_prefix('\u{FEFF}').unwrap_or(raw);
        let normalised = raw.replace("\r\n", "\n");
        let (frontmatter, body) = split_frontmatter(&normalised)
            .ok_or_else(|| format!("skill '{name}': missing or malformed frontmatter"))?;

        let mut fm_name: Option<String> = None;
        let mut fm_description: Option<String> = None;
        let mut fm_review_gate = false;
        let mut fm_globs: Vec<String> = Vec::new();

        // Open-key state: a top-level `key:` with no inline value opens a
        // block list; subsequent indented `- item` lines belong to it.
        // Modeled on directives.rs::field but re-implemented here — those
        // helpers are private and return the wrong shape.
        let mut open_key: Option<String> = None;

        for line in frontmatter.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            // Indented list item under an open block key.
            let indent = line.len() - line.trim_start().len();
            if indent > 0 && trimmed.starts_with('-') && open_key.as_deref() == Some("globs") {
                let item = trimmed[1..].trim().trim_matches('"').trim_matches('\'');
                if !item.is_empty() {
                    fm_globs.push(item.to_string());
                }
                continue;
            }
            if indent > 0 && trimmed.starts_with('-') {
                continue;
            }
            // Indented non-list line (e.g. `globs: x/**` nested under
            // `metadata:`) — belongs to a nested block, never to the
            // top-level key set. Skip it and close any open block key:
            // a nested region means the previous block list is over.
            if indent > 0 {
                open_key = None;
                continue;
            }

            // Top-level key — closes any open block key.
            open_key = None;

            let Some((key, value)) = trimmed.split_once(':') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            // Inline value: strip quoting, then optional `[a, b]` flow form.
            let value = value.trim_matches('"').trim_matches('\'');

            match key {
                "name" => fm_name = Some(value.to_string()),
                "description" => fm_description = Some(value.to_string()),
                "review_gate" => {
                    fm_review_gate = matches!(
                        value.to_ascii_lowercase().as_str(),
                        "true" | "yes" | "1" | "on"
                    );
                }
                "globs" => {
                    if value.is_empty() {
                        // Block list form: `globs:` with items on the
                        // following indented lines.
                        open_key = Some(key.to_string());
                    } else if let Some(inner) =
                        value.strip_prefix('[').and_then(|s| s.strip_suffix(']'))
                    {
                        // Inline flow form: `globs: [a, b]`.
                        for part in inner.split(',') {
                            let item = part.trim().trim_matches('"').trim_matches('\'');
                            if !item.is_empty() {
                                fm_globs.push(item.to_string());
                            }
                        }
                    } else {
                        // Cursor comma-separated string form: `globs: a, b`.
                        for part in value.split(',') {
                            let item = part.trim().trim_matches('"').trim_matches('\'');
                            if !item.is_empty() {
                                fm_globs.push(item.to_string());
                            }
                        }
                    }
                }
                _ => {} // unknown keys (incl. nested metadata blocks) ignored
            }
        }

        let resolved_name = fm_name.unwrap_or_else(|| name.to_string());
        let description = fm_description
            .ok_or_else(|| format!("skill '{name}': frontmatter missing 'description'"))?;
        let slash_name = format!("/{resolved_name}");

        Ok(Self {
            name: resolved_name,
            slash_name,
            description,
            body: body.trim().to_string(),
            globs: fm_globs,
            review_gate: fm_review_gate,
            source,
        })
    }

    /// The prompt dispatched on slash invocation: the skill body with
    /// [`REVIEW_GATE_REMINDER`] prepended when the skill declares
    /// `review_gate: true`. Unflagged skills return the body unchanged.
    pub fn prompt_body(&self) -> String {
        if self.review_gate {
            format!("{REVIEW_GATE_REMINDER}\n\n---\n\n{}", self.body)
        } else {
            self.body.clone()
        }
    }
}

/// Split a `SKILL.md` blob into (frontmatter, body).
///
/// Caller is expected to have normalised line endings to `\n` and stripped
/// any BOM. The opening fence must be the very first line (`---\n`);
/// the closing fence is the next standalone `---` line. Returns `None`
/// if either fence is missing.
pub(crate) fn split_frontmatter(raw: &str) -> Option<(&str, &str)> {
    let after_open = raw.strip_prefix("---\n")?;

    // Closing fence: a line that is exactly "---". Find its byte offset
    // by walking lines and tracking position — `len() + 1` is correct
    // here because we operate on LF-only input.
    let close_idx = after_open
        .lines()
        .scan(0usize, |acc, line| {
            let start = *acc;
            *acc += line.len() + 1;
            Some((start, line))
        })
        .find(|(_, line)| line.trim() == "---")
        .map(|(idx, _)| idx)?;

    let frontmatter = &after_open[..close_idx];
    // Skip past "---\n" to the body (or end-of-string for an empty body).
    let body_start = (close_idx + 4).min(after_open.len());
    let body = &after_open[body_start..];
    Some((frontmatter, body))
}

/// User skills directory: `~/.opencrabs/skills/`.
pub(crate) fn user_skills_dir() -> PathBuf {
    crate::config::opencrabs_home().join("skills")
}

/// Load every available skill (built-ins + project overlays + user overlays).
///
/// Resolution order (last writer wins):
/// 1. Built-in skills shipped with the binary
/// 2. Project-specific skills from `~/.opencrabs/projects/*/skills/`
/// 3. User profile overlay from `~/.opencrabs/skills/`
///
/// Skills with broken frontmatter are skipped with a warning rather than
/// aborting the load.
pub fn load_all_skills() -> Vec<Skill> {
    let mut by_name: std::collections::BTreeMap<String, Skill> = std::collections::BTreeMap::new();

    // 1. Built-ins
    for (name, raw) in BUILTIN_SKILLS {
        match Skill::parse(name, raw, SkillSource::Builtin) {
            Ok(skill) => {
                by_name.insert(skill.name.clone(), skill);
            }
            Err(e) => {
                tracing::error!("skills: built-in '{name}' failed to parse: {e}");
            }
        }
    }

    // 2. Project-specific skills: ~/.opencrabs/projects/*/skills/<name>/SKILL.md
    let projects_dir = crate::services::ProjectService::projects_dir();
    if let Ok(projects) = std::fs::read_dir(&projects_dir) {
        for project_entry in projects.flatten() {
            let skills_dir = project_entry.path().join("skills");
            if !skills_dir.is_dir() {
                continue;
            }
            if let Ok(entries) = std::fs::read_dir(&skills_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_dir() {
                        continue;
                    }
                    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                        continue;
                    };
                    let skill_path = path.join("SKILL.md");
                    if !skill_path.exists() {
                        continue;
                    }
                    let raw = match std::fs::read_to_string(&skill_path) {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!(
                                "skills: failed to read project skill '{name}' at {}: {e}",
                                skill_path.display()
                            );
                            continue;
                        }
                    };
                    match Skill::parse(name, &raw, SkillSource::User) {
                        Ok(skill) => {
                            by_name.insert(skill.name.clone(), skill);
                        }
                        Err(e) => {
                            tracing::warn!(
                                "skills: project skill '{name}' has bad frontmatter: {e}"
                            );
                        }
                    }
                }
            }
        }
    }

    // 3. User profile overlay: ~/.opencrabs/skills/<name>/SKILL.md
    let dir = user_skills_dir();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let skill_path = path.join("SKILL.md");
            if !skill_path.exists() {
                continue;
            }
            let raw = match std::fs::read_to_string(&skill_path) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(
                        "skills: failed to read user skill '{name}' at {}: {e}",
                        skill_path.display()
                    );
                    continue;
                }
            };
            match Skill::parse(name, &raw, SkillSource::User) {
                Ok(skill) => {
                    by_name.insert(skill.name.clone(), skill);
                }
                Err(e) => {
                    tracing::warn!("skills: user skill '{name}' has bad frontmatter: {e}");
                }
            }
        }
    }

    by_name.into_values().collect()
}

/// Look up a single skill by name, applying the same resolution rules as
/// `load_all_skills` (user overlay wins).
pub fn resolve_skill(name: &str) -> Option<Skill> {
    load_all_skills().into_iter().find(|s| s.name == name)
}

type GlobsCache = std::sync::Mutex<Option<(std::time::Instant, PathBuf, Vec<Skill>)>>;
static GLOBS_CACHE: OnceLock<GlobsCache> = OnceLock::new();

/// Clear the globs cache (for tests or explicit reload).
pub fn invalidate_globs_cache() {
    if let Some(cache) = GLOBS_CACHE.get()
        && let Ok(mut guard) = cache.lock()
    {
        *guard = None;
    }
}

/// The skills that declare `globs` — the skill-gate's working set (#150).
/// Cached for 60s: the gate runs on EVERY tool call, and re-scanning the
/// skills tree per call would dominate it. A freshly added globs skill
/// becomes visible within a minute or on restart; failure to read the
/// cache source is fail-open (empty vec → gate passes everything).
pub fn skills_with_globs() -> Vec<Skill> {
    static TTL: std::time::Duration = std::time::Duration::from_secs(60);
    let cache = GLOBS_CACHE.get_or_init(|| std::sync::Mutex::new(None));
    let current_home = crate::config::opencrabs_home();
    let mut guard = cache.lock().expect("skills_with_globs cache poisoned");
    if let Some((at, home, skills)) = guard.as_ref()
        && at.elapsed() < TTL
        && home == &current_home
    {
        return skills.clone();
    }
    let fresh: Vec<Skill> = load_all_skills()
        .into_iter()
        .filter(|s| !s.globs.is_empty())
        .collect();
    *guard = Some((std::time::Instant::now(), current_home, fresh.clone()));
    fresh
}
