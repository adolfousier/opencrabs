//! Skill glob gate (issue #150) — the registry-level sibling of
//! `plan_gate`.
//!
//! Cursor-style opt-in enforcement for skills: a `SKILL.md` that declares
//! a `globs:` frontmatter field guards its own topic. When the agent
//! attempts a tool call that references a path matching one of those
//! globs, and the skill body is NOT loaded (seen) in the current session
//! context — fresh sessions AND post-compaction (owner decision
//! 2026-09-10) — the call is rejected. The rejection's content IS the
//! full skill body (the `plan_gate` deny precedent), so the body lands
//! in-context the same turn and the identical retry succeeds.
//!
//! Laws (design v2, decisions 6–9):
//! - **Fail-open:** any gate-internal error (pattern compile failure,
//!   malformed globs, missing skill file, registry error) → `Pass`. The
//!   gate must never dead-end an agent.
//! - **Opt-in:** no `globs` key → the skill is invisible to the gate.
//! - **Exempt recovery tools** must never be gated, or a blocked agent
//!   cannot self-serve (`load_brain_file`, `read_file`, `slash_command`,
//!   `session_search`, `tool_search`, `write_opencrabs_file`,
//!   `execute_code`).
//! - **Cursor match table:** `*` matches one path segment (never `/`),
//!   `**` matches recursively — `require_literal_separator: true`.
//!   Globs match against the normalized ABSOLUTE path; skill authors
//!   use `**/` prefixes (e.g. `**/skills/opencrabs-dev/**`).
//! - **Pattern resolution** ([`compile_pattern`]): `~` expands to home;
//!   a wildcard-led pattern (`**/x`) is unanchored and matched verbatim;
//!   any other relative pattern anchors at the session cwd. A glob is
//!   never blanket-prefixed with `cwd` — that silently disarms every
//!   documented `**/…` form.
//! - **Cheap + deterministic:** fast-exit when disabled / no loaded
//!   skill declares globs / tool is exempt.

use serde_json::Value;
use uuid::Uuid;

use super::seen_skills;

/// Tools the gate never touches — recovery paths a blocked agent must
/// keep available to read the skill body and re-arm itself.
pub(crate) const EXEMPT_TOOLS: &[&str] = &[
    "load_brain_file",
    "read_file",
    "slash_command",
    "session_search",
    "tool_search",
    "write_opencrabs_file",
    "execute_code",
];

/// Input keys harvested as candidate paths (decision 6). `grep`'s
/// `pattern` is excluded (regex, not a path); the `glob` tool's
/// `pattern` is excluded (a glob pattern string is not a path — finding
/// 24). Bash commands are harvested separately as path-like tokens.
const PATH_KEYS: &[&str] = &["path", "file_path", "filePath"];

/// Tools whose `pattern`-like keys are NOT harvested even when named in
/// [`PATH_KEYS`]-adjacent forms. (Kept explicit for the doc contract.)
const PATTERN_ONLY_TOOLS: &[&str] = &["grep", "glob"];

/// The gate's verdict on one tool call.
#[derive(Debug, Clone, PartialEq)]
pub enum GateVerdict {
    /// The call proceeds (also the verdict for every fail-open path).
    Pass,
    /// The call is blocked: the named skill's body rides the rejection.
    Block {
        skill: String,
        matched_path: String,
        body: String,
        globs: Vec<String>,
    },
}

/// Gate entry point. `enabled` is the `[agent] skill_glob_gate` master
/// switch; `cwd` is the session's working directory for resolving
/// relative candidate paths.
pub fn check(
    session_id: Uuid,
    tool_name: &str,
    input: &Value,
    cwd: &std::path::Path,
    enabled: bool,
) -> GateVerdict {
    if !enabled || EXEMPT_TOOLS.contains(&tool_name) {
        return GateVerdict::Pass;
    }
    // Fast-exit: no loaded skill declares globs (decision 12 — zero cost).
    let skills = crate::brain::skills::skills_with_globs();
    if skills.is_empty() {
        return GateVerdict::Pass;
    }
    let candidates = harvest_candidates(tool_name, input, cwd);
    if candidates.is_empty() {
        return GateVerdict::Pass;
    }

    for skill in &skills {
        for glob_str in &skill.globs {
            let pattern = match compile_pattern(glob_str, cwd) {
                Ok(p) => p,
                Err(e) => {
                    // Malformed glob: warn once per (slug, glob) per
                    // process and skip — fail-open (decision 9).
                    warn_malformed_glob(&skill.name, glob_str, &e.to_string());
                    continue;
                }
            };
            let options = glob::MatchOptions {
                case_sensitive: false,
                require_literal_separator: true, // Cursor `*` = one segment (B5)
                require_literal_leading_dot: false,
            };
            for candidate in &candidates {
                if pattern.matches_path_with(candidate, options) {
                    if seen_skills::seen_since_compaction(session_id, &skill.name) {
                        return GateVerdict::Pass;
                    }
                    return GateVerdict::Block {
                        skill: skill.name.clone(),
                        matched_path: candidate.display().to_string(),
                        body: skill.prompt_body(),
                        globs: skill.globs.clone(),
                    };
                }
            }
        }
    }
    GateVerdict::Pass
}

/// Warn-once set for malformed globs (decision 9).
fn warn_malformed_glob(slug: &str, glob_str: &str, err: &str) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNT: AtomicU64 = AtomicU64::new(0);
    // Warn at most 8 times per process per signature to bound log noise
    // without adding a second map.
    if COUNT.fetch_add(1, Ordering::Relaxed) < 8 {
        tracing::warn!(
            "skill_gate: skill '{slug}' has malformed glob '{glob_str}' ({err}) — skipping (fail-open)"
        );
    }
}

/// Harvest candidate absolute paths from the tool input (decision 6).
pub(crate) fn harvest_candidates(
    tool_name: &str,
    input: &Value,
    cwd: &std::path::Path,
) -> Vec<std::path::PathBuf> {
    let mut out: Vec<String> = Vec::new();
    let obj = match input.as_object() {
        Some(o) => o,
        None => return Vec::new(),
    };

    if !PATTERN_ONLY_TOOLS.contains(&tool_name) {
        for key in PATH_KEYS {
            if let Some(Value::String(s)) = obj.get(*key) {
                out.push(s.clone());
            }
        }
    } else {
        // grep/glob: the `pattern` key is regex/glob text — never a path —
        // but their `path` argument IS a real search root and must still be
        // harvested, otherwise a gated skill under a grep'd directory never
        // fires. Only `pattern` is excluded (finding 24's actual scope).
        if let Some(Value::String(p)) = obj.get("path") {
            out.push(p.clone());
        }
    }

    // Bash: extract path-like tokens — words containing `/` or tilde
    // prefixes. Leaky by design (fail-open): we prefer deterministic-cheap
    // over exhaustive.
    if tool_name == "bash"
        && let Some(Value::String(cmd)) = obj.get("command")
    {
        for token in cmd.split_whitespace() {
            let tok = token.trim_matches(|c| c == '"' || c == '\'' || c == ';' || c == ',');
            if tok.contains('/') || tok.starts_with('~') {
                out.push(tok.to_string());
            }
        }
    }

    out.into_iter()
        .map(|raw| {
            let expanded = super::error::expand_tilde(&raw);
            let joined = if expanded.is_absolute() {
                expanded
            } else {
                cwd.join(expanded)
            };
            normalize(&joined)
        })
        .collect()
}

/// Compile a frontmatter glob into a matcher.
///
/// A glob is not a path, so the resolution is deliberately narrow — and the
/// narrowness is load-bearing:
///
/// 1. a leading `~` expands to the home directory (`~/repo/**`). Without
///    this the natural Cursor-style form compiles with a literal `~` and can
///    never match an absolute candidate path: the gate would be silently
///    inert for exactly the skills that opt in.
/// 2. a pattern that is absolute, or whose first component starts with a
///    wildcard (`**/x`, `*/x`, `[ab]/x`), is used VERBATIM. `**/…` is an
///    *unanchored* "match anywhere" pattern (`**/test` matches
///    `/one/two/test`); prefixing it with `cwd` pins it to a prefix it can
///    never satisfy and disarms the gate for every such skill.
/// 3. any other relative pattern (`src/**`) anchors at the tool's `cwd`,
///    mirroring how relative candidate paths are resolved — so a
///    root-relative author intent still matches, instead of failing open.
/// 4. the result is normalized (`.` / `..` / duplicate separators).
pub(crate) fn compile_pattern(
    glob_str: &str,
    cwd: &std::path::Path,
) -> Result<glob::Pattern, glob::PatternError> {
    let expanded = super::error::expand_tilde(glob_str);
    let anchored = if expanded.is_absolute() || is_wildcard_led(glob_str) {
        expanded
    } else {
        cwd.join(expanded)
    };
    glob::Pattern::new(&normalize(&anchored).to_string_lossy())
}

/// Whether a glob's first path component starts with a wildcard — i.e. the
/// pattern is anchored nowhere and must match at any depth.
fn is_wildcard_led(glob_str: &str) -> bool {
    matches!(glob_str.as_bytes().first(), Some(b'*' | b'?' | b'['))
}
/// Normalize a path for matching: resolve `.` / `..` lexically, strip
/// redundant separators. No filesystem access — purely string-level so
/// the gate cannot fail on missing paths.
fn normalize(path: &std::path::Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for comp in path.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}
