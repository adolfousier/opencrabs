//! Load Brain File Tool
//!
//! Loads a specific brain context file from `~/.opencrabs/` on demand.
//! Use this to fetch USER.md, MEMORY.md, AGENTS.md, etc. only when the
//! current request actually needs that context, rather than injecting all
//! files into every turn.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

use crate::brain::prompt_builder::CONTEXTUAL_BRAIN_FILES;

pub struct LoadBrainFileTool;

#[async_trait]
impl Tool for LoadBrainFileTool {
    fn name(&self) -> &str {
        "load_brain_file"
    }

    fn description(&self) -> &str {
        "Load any .md file from your OpenCrabs home directory (see Known paths — profile-scoped). \
         Works with built-in files (USER.md, MEMORY.md, AGENTS.md, TOOLS.md, SECURITY.md) \
         and any custom .md files you have created in your workspace. \
         Pass name=\"all\" to load all .md files at once. \
         Pass an optional `query` to get back ONLY the sections that match, instead of the whole \
         file — use it when you want to check what the rules say about something specific \
         (e.g. name=\"MEMORY.md\", query=\"telegram owner gate\"). Cheap, so prefer it over \
         loading a large file in full. \
         To edit or update brain files, use the `write_opencrabs_file` tool."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Brain file to load, e.g. \"MEMORY.md\", \"USER.md\", \"AGENTS.md\", \"TOOLS.md\", \"SECURITY.md\". Use \"all\" to load all contextual files."
                },
                "query": {
                    "type": "string",
                    "description": "Optional. Return only the sections of the file matching this text, instead of the whole file. Whole sections are returned, so a rule is never cut in half."
                }
            },
            "required": ["name"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::ReadFiles]
    }

    fn requires_approval(&self) -> bool {
        false
    }

    async fn execute(&self, input: Value, ctx: &ToolExecutionContext) -> Result<ToolResult> {
        let name = input
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();

        if name.is_empty() {
            return Ok(ToolResult::error("name parameter is required".to_string()));
        }

        // Optional: return only matching sections rather than the whole file.
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();

        // Skill-slug form (issue #131): a bare skill name (no `.md`, no
        // separators) resolves through the skill registry — same rules as
        // slash invocation — and returns the prompt body. This gives
        // agents a sanctioned reload surface that works where the raw
        // filename form cannot (skills live in subdirectories). A success
        // here marks the skill as SEEN for this session, feeding the
        // post-compaction inventory stamp (#125/#131 union).
        // A bare name that resolves to no known skill falls through to the
        // brain-file handling below, which produces the "not found" error.
        if !name.ends_with(".md")
            && !name.contains('/')
            && !name.contains('\\')
            && let Some(skill) = crate::brain::skills::resolve_skill(name)
        {
            super::seen_skills::mark_seen(ctx.session_id, &skill.name);
            tracing::info!(
                "load_brain_file: resolved skill slug '{}', marked seen for session {}",
                skill.name,
                ctx.session_id
            );
            let body = skill.prompt_body();
            return Ok(ToolResult::success(if query.is_empty() {
                format!("--- skill: {} ---\n{}", skill.name, body)
            } else {
                let matches = crate::brain::brain_sections::find_sections(&body, query);
                tracing::info!(
                    "load_brain_file(skill {}): query={query:?}: {} section(s) returned",
                    skill.name,
                    matches.sections.len()
                );
                matches.render(&format!("skill: {}", skill.name), query)
            }));
        }

        let home = crate::config::opencrabs_home();

        // Filename form of a skill (issue #138): "<slug>.md" resolves through
        // the skill registry exactly like the bare slug form — same marking,
        // same body framing, query filtering included. Before #138 this form
        // could only miss (skills live in skills/<slug>/SKILL.md, never in the
        // home dir flat namespace), and a query-filtered miss registered
        // nothing — the exact undercount the #138 probe caught live.
        //
        // A real brain file wins. The whole premise above is that this form
        // could only ever MISS, so it may only claim names that still miss:
        // the home file is a user artifact, the skill is shipped, and silently
        // handing back skill text for a file the user wrote is a lie no log
        // line makes visible. The existence check mirrors the resolution
        // below — plain `home.join(name)`, since a kebab-lowercase skill slug
        // never matches a CONTEXTUAL_BRAIN_FILES entry case-insensitively.
        if let Some(skill) = name
            .strip_suffix(".md")
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.contains('/') && !s.contains('\\'))
            .filter(|_| !home.join(name).exists())
            .and_then(crate::brain::skills::resolve_skill)
        {
            super::seen_skills::mark_seen(ctx.session_id, &skill.name);
            tracing::info!(
                "load_brain_file: resolved filename form '{}.md' to skill '{}', marked seen \
                 for session {}",
                skill.name,
                skill.name,
                ctx.session_id
            );
            let body = skill.prompt_body();
            return Ok(ToolResult::success(if query.is_empty() {
                format!("--- skill: {} ---\n{}", skill.name, body)
            } else {
                let matches = crate::brain::brain_sections::find_sections(&body, query);
                tracing::info!(
                    "load_brain_file(skill {}, filename form): query={query:?}: {} section(s) \
                     returned",
                    skill.name,
                    matches.sections.len()
                );
                matches.render(&format!("skill: {}", skill.name), query)
            }));
        }

        // Read-time empty-section stripping. Default on; opt out via
        // `[brain] strip_empty_sections = false` in config.toml.
        // Disk stays authoritative — writes never run through this.
        let strip_enabled = crate::config::Config::current().brain.strip_empty_sections;
        let apply_filter = |raw: String| -> (String, Vec<String>) {
            if !strip_enabled {
                return (raw, Vec::new());
            }
            let res = crate::brain::filter::strip_empty_sections(&raw);
            (res.content, res.stripped_headers)
        };

        // Per-project brain overlay. If this session belongs to a project that
        // carries its own brain file, it loads ON TOP of the profile's file
        // (append, never replace) so a project can ADD context without ever
        // silently dropping a profile-level hard rule. `None` for sessions with
        // no project, CLI one-shots, or tests without a service_context.
        let project_overlay: Option<(String, std::path::PathBuf)> = match &ctx.service_context {
            Some(svc) => {
                crate::services::ProjectService::new(svc.clone())
                    .project_brain_dir(ctx.session_id)
                    .await
            }
            None => None,
        };
        // Format a project overlay section for `fname`, or `None` if no project,
        // no such overlay file, or it filters down to empty.
        let overlay_section = |fname: &str| -> Option<String> {
            let (pname, dir) = project_overlay.as_ref()?;
            let raw = std::fs::read_to_string(dir.join(fname)).ok()?;
            let filtered = if strip_enabled {
                crate::brain::filter::strip_empty_sections(&raw).content
            } else {
                raw
            };
            let trimmed = filtered.trim();
            if trimmed.is_empty() {
                return None;
            }
            Some(format!(
                "--- {} (project: {} overlay) ---\n{}\n\n",
                fname, pname, trimmed
            ))
        };

        if name == "all" {
            let mut out = String::new();
            let mut stripped_all: Vec<String> = Vec::new();
            let mut seen = std::collections::HashSet::new();

            // Known contextual files first (stable order)
            for (fname, label) in CONTEXTUAL_BRAIN_FILES {
                seen.insert(fname.to_lowercase());
                let path = home.join(fname);
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let (filtered, stripped) = apply_filter(content);
                    let trimmed = filtered.trim();
                    if !trimmed.is_empty() {
                        out.push_str(&format!("--- {} ({}) ---\n{}\n\n", fname, label, trimmed));
                    }
                    for h in stripped {
                        stripped_all.push(format!("{}: {}", fname, h));
                    }
                }
                // Project overlay rides ON TOP, right after the profile file.
                if let Some(ov) = overlay_section(fname) {
                    out.push_str(&ov);
                }
            }

            // User-created .md files not in the known list
            if let Ok(entries) = std::fs::read_dir(&home) {
                let mut extras: Vec<_> = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        let name = e.file_name().to_string_lossy().to_string();
                        name.ends_with(".md") && !seen.contains(&name.to_lowercase())
                    })
                    .collect();
                extras.sort_by_key(|e| e.file_name());
                for entry in extras {
                    let fname = entry.file_name().to_string_lossy().to_string();
                    if let Ok(content) = std::fs::read_to_string(entry.path()) {
                        let (filtered, stripped) = apply_filter(content);
                        let trimmed = filtered.trim();
                        if !trimmed.is_empty() {
                            out.push_str(&format!("--- {} (user) ---\n{}\n\n", fname, trimmed));
                        }
                        for h in stripped {
                            stripped_all.push(format!("{}: {}", fname, h));
                        }
                    }
                    // Project overlay rides ON TOP, right after the profile file.
                    if let Some(ov) = overlay_section(&fname) {
                        out.push_str(&ov);
                    }
                    seen.insert(fname.to_lowercase());
                }
            }

            // Project-only brain files (no profile counterpart) still ride ON TOP.
            if let Some((_, dir)) = &project_overlay
                && let Ok(entries) = std::fs::read_dir(dir)
            {
                let mut extras: Vec<_> = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        let n = e.file_name().to_string_lossy().to_string();
                        n.ends_with(".md") && !seen.contains(&n.to_lowercase())
                    })
                    .collect();
                extras.sort_by_key(|e| e.file_name());
                for entry in extras {
                    let fname = entry.file_name().to_string_lossy().to_string();
                    if let Some(ov) = overlay_section(&fname) {
                        out.push_str(&ov);
                    }
                    seen.insert(fname.to_lowercase());
                }
            }

            if !stripped_all.is_empty() {
                tracing::info!(
                    "load_brain_file(all): stripped {} empty section(s) on read: {:?}",
                    stripped_all.len(),
                    stripped_all
                );
            }

            return if out.is_empty() {
                Ok(ToolResult::success("No brain files found.".to_string()))
            } else {
                Ok(ToolResult::success(out))
            };
        }

        // Validate filename: must be a simple .md name (no path traversal)
        if name.contains('/') || name.contains('\\') || name.contains("..") {
            return Ok(ToolResult::error(format!(
                "Invalid brain file name '{}'. Must be a simple filename with no path separators",
                name
            )));
        }

        // Use canonical casing from the known list if it matches, otherwise use as-is
        let canonical = CONTEXTUAL_BRAIN_FILES
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(n, _)| *n)
            .unwrap_or(name);

        let path = home.join(canonical);
        let overlay = overlay_section(canonical);
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let (filtered, stripped) = apply_filter(content);
                if !stripped.is_empty() {
                    tracing::info!(
                        "load_brain_file({}): stripped {} empty section(s) on read: {:?}",
                        canonical,
                        stripped.len(),
                        stripped
                    );
                }
                let trimmed = filtered.trim();

                // A query returns only the matching sections (#800). Runs over
                // the profile file AND its project overlay, since that is what
                // loading the file in full would have given.
                if !query.is_empty() {
                    let mut combined = trimmed.to_string();
                    if let Some(ref ov) = overlay {
                        if !combined.is_empty() {
                            combined.push_str("\n\n");
                        }
                        combined.push_str(ov.trim_end());
                    }
                    let matches = crate::brain::brain_sections::find_sections(&combined, query);
                    tracing::info!(
                        "load_brain_file({canonical}) query={query:?}: {} section(s) returned, {} omitted",
                        matches.sections.len(),
                        matches.omitted
                    );
                    return Ok(ToolResult::success(matches.render(canonical, query)));
                }

                let mut out = if trimmed.is_empty() {
                    String::new()
                } else {
                    format!("--- {} ---\n{}", canonical, trimmed)
                };
                if let Some(ov) = overlay {
                    if !out.is_empty() {
                        out.push_str("\n\n");
                    }
                    out.push_str(ov.trim_end());
                }
                if out.is_empty() {
                    Ok(ToolResult::success(format!(
                        "{} exists but is empty.",
                        canonical
                    )))
                } else {
                    Ok(ToolResult::success(out))
                }
            }
            // Profile file missing — still surface a project overlay if one exists.
            Err(_) => match overlay {
                Some(ov) => Ok(ToolResult::success(ov.trim_end().to_string())),
                None => Ok(ToolResult::success(format!(
                    "{} not found in your OpenCrabs home ({}). No content available.",
                    canonical, canonical
                ))),
            },
        }
    }
}
