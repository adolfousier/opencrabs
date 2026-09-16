//! Self-Improve Tool — Recursive Self-Improvement (RSI)
//!
//! Autonomously applies improvements to brain files based on feedback analysis.
//! Writes to ~/.opencrabs/rsi/ directory — no human approval required.
//! Each improvement is logged to rsi/improvements.md and archived daily in rsi/history/.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::io::Write;

use crate::brain::tools::brain_verify::GateDecision;

/// OODA **Orient** stage for the autonomous RSI path: verify the *proposed*
/// brain-file content against the belief base (`brain_verify.toml`) BEFORE any
/// write, and hard-fail when no belief base is loaded (no silent no-op). This
/// is the contract gate `write_opencrabs_file` enforces on the user-facing path
/// but `self_improve` previously bypassed (#881: RSI was the sole verifier of
/// its own writes).
fn orient_gate(target_file: &str, proposed_content: &str) -> GateDecision {
    crate::brain::tools::brain_verify::orient_gate_decision_active(target_file, proposed_content)
}

/// OODA **Verify** stage: re-read the file after a write and confirm the change
/// actually persisted. Catches the overwrite/sync race where a write succeeds
/// but the change vanishes (e.g. a concurrent template-sync regenerates the
/// file). The marker is a snippet guaranteed to be in the just-written content.
/// (#881)
fn verify_persisted(
    target_path: &std::path::Path,
    marker: &str,
) -> std::result::Result<(), String> {
    match std::fs::read_to_string(target_path) {
        Ok(after) if after.contains(marker) => Ok(()),
        Ok(_) => Err(format!(
            "Verify stage: wrote to {} but the change did not persist on re-read \
             (the file may have been overwritten by a concurrent sync)",
            target_path.display()
        )),
        Err(e) => Err(format!(
            "Verify stage: could not re-read {} after write: {e}",
            target_path.display()
        )),
    }
}

/// Ensures the RSI directory structure exists.
fn ensure_rsi_dirs(home: &std::path::Path) -> std::io::Result<()> {
    let rsi_dir = home.join("rsi");
    let history_dir = rsi_dir.join("history");
    std::fs::create_dir_all(&history_dir)
}

/// Known brain files that the RSI tool is allowed to read/modify.
const ALLOWED_FILES: &[&str] = &[
    "SOUL.md",
    "USER.md",
    "AGENTS.md",
    "TOOLS.md",
    "CODE.md",
    "SECURITY.md",
    "MEMORY.md",
    "BOOT.md",
];

/// Heuristic guard: reject brain-file content that looks like a raw failure
/// event log (timestamps, session IDs, `(N failures: ...)` counters in
/// section headers) rather than a derived rule. Issue #111: RSI cycles were
/// appending sections like `### Timeout Handling (5 failures: 17:02, 16:59,
/// 16:58, 16:57, 16:55)` to TOOLS.md, turning the brain file into an audit
/// log instead of operational guidance.
///
/// The system prompt tells the agent not to do this; this guard catches
/// the cases where the model ignores the prompt anyway.
///
/// Returns `Some(reason)` when the content should be rejected, `None`
/// when it passes.
fn looks_like_failure_log(content: &str) -> Option<&'static str> {
    for line in content.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with('#') {
            // Check plain-text lines for incident-log patterns.
            // Catches entries like:
            //   "ADDED 2026-06-11 (session ba623fd1): Another quoting error..."
            //   "REPEAT 2026-06-13 (session ca91e02d): SSH quoting violation again..."
            let lower = trimmed.to_ascii_lowercase();
            if (lower.starts_with("added ") || lower.starts_with("repeat "))
                && lower.contains("session ")
                && trimmed.chars().any(|c| c.is_ascii_digit())
            {
                return Some(
                    "Plain-text incident-log entry detected \
                     (e.g. 'ADDED YYYY-MM-DD (session ...): ...'). Brain files hold \
                     derived RULES, not raw incident logs. Use the feedback ledger \
                     (feedback_analyze) for incident history. Document the rule itself \
                     and mention the feedback data source, not individual dated entries.",
                );
            }
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        // Section headers describing a failure count: "(N failures:",
        // "(N failures since", "— N failures", etc.
        if (lower.contains("failures:") || lower.contains("failures since"))
            && trimmed.chars().any(|c| c.is_ascii_digit())
        {
            return Some(
                "Section header looks like a failure-event log \
                 (e.g. `### Foo (N failures: ...)`). Brain files hold derived RULES, \
                 not raw audit data. Restate the header as the rule itself and put \
                 any incident counter on a single inline line in the body \
                 (e.g. `Violations: 6`). See issue #111.",
            );
        }
        // "— Recurring (N failures since YYYY-MM-DD)" — same shape with different prefix.
        if lower.contains("recurring") && lower.contains("failures") {
            return Some(
                "`Recurring (... failures ...)` headers are audit-log entries, \
                 not operational rules. Restate the section as the cause-and-fix \
                 rule the agent should follow; do not list dates/sessions in the header.",
            );
        }
    }
    None
}

/// Check if content is trivial/meaningless (test entries, single words, etc.)
fn is_trivial_content(content: &str, description: &str) -> bool {
    let c = content.trim();
    let d = description.trim().to_ascii_lowercase();

    // Single word or very short content
    if c.len() < 15 && !c.contains('\n') {
        return true;
    }

    // Literally just "test" or similar
    if matches!(d.as_str(), "test" | "testing" | "test entry" | "test test") {
        return true;
    }

    // Content is just the word "test" repeated
    if c.eq_ignore_ascii_case("test") {
        return true;
    }

    false
}

pub struct SelfImproveTool;

#[async_trait]
impl Tool for SelfImproveTool {
    fn name(&self) -> &str {
        "self_improve"
    }

    fn description(&self) -> &str {
        "Autonomously apply self-improvements based on feedback analysis. \
         Modifies brain files (SOUL.md, AGENTS.md, etc.) and logs changes to \
         your `rsi/improvements.md`. No human approval needed — the agent \
         identifies patterns via feedback_analyze and applies fixes directly. \
         Use feedback_analyze first to identify what needs improvement."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "What to do:\n\
                        - 'read': Read a brain file BEFORE modifying it. ALWAYS do this first, and \
                        search what you read for the rule you are about to write. If it is already \
                        there in any wording, do NOT append a second copy — either leave it alone or \
                        'update' the existing one.\n\
                        - 'apply': Append a genuinely NEW rule. Keep it to ONE directive, under 600 \
                        characters. These files are loaded on every turn of every channel, so the \
                        cost is permanent. Put the rule in a bold lead and stop; incident history \
                        belongs in the feedback ledger, and a 'Violations: N' count is enough here. \
                        Over-length is rejected, and splitting one rule across two appends to dodge \
                        that is the same mistake twice.\n\
                        - 'update': Surgically replace an existing section/paragraph. Use when an \
                        existing instruction needs refinement rather than a new one added, and to \
                        CONSOLIDATE: a long rule may be replaced by a shorter version of itself, \
                        provided the replacement keeps the original's bold lead or heading so the \
                        rule is still identifiable. That is the only way these files ever shrink; \
                        it cannot be used to remove a rule.\n\
                        - 'list': Show previously applied improvements.\n\
                        - 'sync_templates': Fetch upstream brain file templates from the repo and append new sections.",
                    "enum": ["read", "apply", "update", "list", "sync_templates"]
                },
                "target_file": {
                    "type": "string",
                    "description": "Brain file to read/modify (e.g. 'SOUL.md', 'TOOLS.md'). Must be a known brain file."
                },
                "description": {
                    "type": "string",
                    "description": "For 'apply'/'update': human-readable description of the improvement"
                },
                "rationale": {
                    "type": "string",
                    "description": "For 'apply'/'update': why this improvement is needed (reference feedback data)"
                },
                "content": {
                    "type": "string",
                    "description": "For 'apply': new content to append. For 'update': the replacement content."
                },
                "old_content": {
                    "type": "string",
                    "description": "For 'update' only: the existing text to find and replace (must be an exact match of the current content)."
                }
            },
            "required": ["action"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::WriteFiles]
    }

    fn requires_approval(&self) -> bool {
        false // Autonomous — no human-in-the-loop
    }

    fn requires_approval_for_input(&self, _input: &Value) -> bool {
        false
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let action = input.get("action").and_then(|v| v.as_str()).unwrap_or("");

        // Brain files always go to ~/.opencrabs/, never the working directory.
        let home = crate::config::opencrabs_home();

        match action {
            "read" => {
                let target_file = input
                    .get("target_file")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if target_file.is_empty() {
                    return Ok(ToolResult::error(
                        "target_file is required for 'read'".to_string(),
                    ));
                }
                if !ALLOWED_FILES.contains(&target_file) {
                    return Ok(ToolResult::error(format!(
                        "target_file must be one of: {}",
                        ALLOWED_FILES.join(", ")
                    )));
                }

                let target_path = home.join(target_file);
                if !target_path.exists() {
                    return Ok(ToolResult::success(format!(
                        "{target_file} does not exist yet (empty). \
                         You can create it with action='apply'."
                    )));
                }
                match std::fs::read_to_string(&target_path) {
                    Ok(content) => Ok(ToolResult::success(format!(
                        "--- {target_file} ({} bytes) ---\n{content}",
                        content.len()
                    ))),
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to read {target_file}: {e}"
                    ))),
                }
            }

            "list" => {
                let improvements_path = home.join("rsi").join("improvements.md");
                if !improvements_path.exists() {
                    return Ok(ToolResult::success(
                        "No improvements recorded yet. Run self_improve with action='apply' to start.".to_string(),
                    ));
                }
                match std::fs::read_to_string(&improvements_path) {
                    Ok(content) => Ok(ToolResult::success(content)),
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to read rsi/improvements.md: {e}"
                    ))),
                }
            }

            "update" => {
                let target_file = input
                    .get("target_file")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let description = input
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let rationale = input
                    .get("rationale")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let old_content = input
                    .get("old_content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let new_content = input.get("content").and_then(|v| v.as_str()).unwrap_or("");

                if target_file.is_empty()
                    || old_content.is_empty()
                    || new_content.is_empty()
                    || description.is_empty()
                {
                    return Ok(ToolResult::error(
                        "target_file, description, old_content, and content are all required for 'update'"
                            .to_string(),
                    ));
                }
                if !ALLOWED_FILES.contains(&target_file) {
                    return Ok(ToolResult::error(format!(
                        "target_file must be one of: {}",
                        ALLOWED_FILES.join(", ")
                    )));
                }

                if let Some(reason) = looks_like_failure_log(new_content) {
                    return Ok(ToolResult::error(reason.to_string()));
                }
                if let Some(reason) = super::self_improve_guards::bans_builtin_tool(new_content) {
                    return Ok(ToolResult::error(reason));
                }

                let target_path = home.join(target_file);
                let existing = match std::fs::read_to_string(&target_path) {
                    Ok(c) => c,
                    Err(_) => {
                        return Ok(ToolResult::error(format!(
                            "{target_file} does not exist — use 'apply' to create new content instead."
                        )));
                    }
                };

                // Find the old_content in the file (exact substring match).
                // The agent is responsible for providing an accurate old_content
                // snippet after reading the file with action='read'.
                if !existing.contains(old_content) {
                    return Ok(ToolResult::error(format!(
                        "old_content not found in {target_file}. \
                         Use action='read' first to get the exact current content, \
                         then copy the section you want to replace verbatim into old_content."
                    )));
                }

                // Perform the replacement (first occurrence only)
                let updated = existing.replacen(old_content, new_content.trim(), 1);

                // Append-only enforcement: brain files are append-only by user
                // policy. Note: cleanup_intent is always false here because RSI is autonomous
                // and cannot get user approval for destructive operations.
                use crate::brain::tools::brain_file_safety;
                // A surgical replace MAY shorten, but only when the rule
                // survives identifiably: `new_content` must still carry
                // `old_content`'s leading directive (#858). That makes this a
                // tightening of one rule, never a section deletion, and
                // check_no_shrink caps the bytes regardless of what we claim.
                //
                // Without this RSI could not consolidate at all: rewording is
                // a shrink, so the files could only ever grow.
                let consolidation =
                    brain_file_safety::is_rule_consolidation(old_content, new_content);
                if let brain_file_safety::ShrinkCheck::Rejected { message } =
                    brain_file_safety::check_no_shrink(
                        &target_path,
                        &existing,
                        &updated,
                        false, // cleanup_intent: RSI cannot do cleanup (no approval mechanism)
                        consolidation,
                    )
                {
                    return Ok(ToolResult::error(message));
                }

                // Ensure RSI dirs exist for logging
                ensure_rsi_dirs(&home).map_err(|e| {
                    crate::brain::tools::ToolError::Execution(format!(
                        "Failed to create RSI directories: {e}"
                    ))
                })?;

                // OODA Orient stage: verify the proposed full content against the
                // belief base before any write. Replaces the bypass (#881).
                match orient_gate(target_file, &updated) {
                    GateDecision::Reject(reason) => {
                        crate::db::repository::AnalyticsEventRepository::emit_brain_verify(
                            target_file,
                            "fail_closed",
                            Some(&reason),
                        );
                        return Ok(ToolResult::error(format!(
                            "{reason}. Change not applied (no write made)."
                        )));
                    }
                    GateDecision::Allow => {
                        crate::db::repository::AnalyticsEventRepository::emit_brain_verify(
                            target_file,
                            "pass",
                            None,
                        );
                    }
                }

                // Snapshot the file before mutating so a bad agent edit can
                // be rolled back from `<file>.YYYY-MM-DDTHHMMSS.bak`.
                if let Err(e) = brain_file_safety::backup_before_write(&target_path) {
                    tracing::warn!("RSI: failed to back up {target_file} before update: {e}");
                }

                // Write the updated file
                std::fs::write(&target_path, updated.as_bytes()).map_err(|e| {
                    crate::brain::tools::ToolError::Execution(format!(
                        "Failed to write {target_file}: {e}"
                    ))
                })?;

                // OODA Verify stage: re-read to confirm the change persisted
                // (overwrite/sync race — #881).
                if let Err(msg) = verify_persisted(&target_path, new_content.trim()) {
                    return Ok(ToolResult::error(format!(
                        "{msg}. The write may have been clobbered — re-apply after the sync settles."
                    )));
                }

                // Log to rsi/improvements.md
                let entry = format!(
                    "\n## [Updated] {}\n\n**Date:** {}\n**Target:** {}\n**Rationale:** {}\n**Status:** Updated (surgical replace)\n",
                    description,
                    chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"),
                    target_file,
                    if rationale.is_empty() {
                        "(none)"
                    } else {
                        rationale
                    },
                );
                match std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(home.join("rsi").join("improvements.md"))
                {
                    Ok(mut f) => {
                        if let Err(e) = f.write_all(entry.as_bytes()) {
                            tracing::warn!("RSI: failed to write improvements.md: {e}");
                        }
                    }
                    Err(e) => {
                        tracing::warn!("RSI: failed to open improvements.md: {e}");
                    }
                }

                // Archive to daily history file
                let history_path = home
                    .join("rsi")
                    .join("history")
                    .join(format!("{}.md", chrono::Utc::now().format("%Y-%m-%d")));
                match std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&history_path)
                {
                    Ok(mut f) => {
                        if let Err(e) = f.write_all(
                            format!(
                                "\n### [Updated] {description}\n\n**Replaced:**\n```\n{old_content}\n```\n**With:**\n```\n{new_content}\n```\n"
                            )
                            .as_bytes(),
                        ) {
                            tracing::warn!("RSI: failed to write history archive: {e}");
                        }
                    }
                    Err(e) => {
                        tracing::warn!("RSI: failed to open history archive: {e}");
                    }
                }

                // Record in feedback ledger
                if let Some(ref svc_ctx) = context.service_context {
                    let repo = crate::db::repository::FeedbackLedgerRepository::new(
                        svc_ctx.pool().clone(),
                    );
                    let meta = serde_json::json!({
                        "target_file": target_file,
                        "rationale": rationale,
                        "action": "update",
                    })
                    .to_string();
                    if let Err(e) = repo
                        .record(
                            &context.session_id.to_string(),
                            "improvement_applied",
                            description,
                            1.0,
                            Some(&meta),
                        )
                        .await
                    {
                        tracing::warn!("RSI: failed to record improvement in feedback ledger: {e}");
                    }
                }

                Ok(ToolResult::success(format!(
                    "Surgically updated {target_file} and logged to rsi/improvements.md: {description}"
                )))
            }

            "apply" => {
                let target_file = input
                    .get("target_file")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let description = input
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let rationale = input
                    .get("rationale")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let content = input.get("content").and_then(|v| v.as_str()).unwrap_or("");

                if target_file.is_empty() || content.is_empty() || description.is_empty() {
                    return Ok(ToolResult::error(
                        "target_file, description, and content are required for 'apply'"
                            .to_string(),
                    ));
                }

                if !ALLOWED_FILES.contains(&target_file) {
                    return Ok(ToolResult::error(format!(
                        "target_file must be one of: {}",
                        ALLOWED_FILES.join(", ")
                    )));
                }

                if let Some(reason) = looks_like_failure_log(content) {
                    return Ok(ToolResult::error(reason.to_string()));
                }
                if let Some(reason) = super::self_improve_guards::bans_builtin_tool(content) {
                    return Ok(ToolResult::error(reason));
                }

                if is_trivial_content(content, description) {
                    return Ok(ToolResult::error(
                        "Content too short or trivial (e.g. 'test').                          Brain files store meaningful rules and context,                          not placeholder text. Provide real content."
                            .to_string(),
                    ));
                }

                // Ensure RSI dirs exist
                ensure_rsi_dirs(&home).map_err(|e| {
                    crate::brain::tools::ToolError::Execution(format!(
                        "Failed to create RSI directories: {e}"
                    ))
                })?;

                let target_path = home.join(target_file);

                // Dedup guard. RSI re-proposes the SAME improvements every cycle
                // (a tool's failure rate doesn't drop just because the guideline
                // was already written), so without this each cycle blindly
                // appended a duplicate paragraph — growing the brain file until
                // the dedup-scan cleaned it up, an endless append→dedup→append
                // loop. Append only genuinely-new paragraphs; if the improvement
                // is already present, skip the write entirely (and don't log it
                // as a fresh "Applied" improvement below).
                use crate::brain::tools::brain_file_safety::{
                    AppendDedup, MAX_RULE_CHARS, filter_duplicate_append,
                };

                // A rule is a one-liner, not an essay (#857). These land in
                // always-loaded files, so every character is paid on every turn
                // of every channel, permanently. Rejected rather than
                // truncated: the model must tighten the rule itself, and
                // splitting it across two appends defeats the point.
                let rule_chars = content.trim().chars().count();
                if rule_chars > MAX_RULE_CHARS {
                    return Ok(ToolResult::error(format!(
                        "Rule is {rule_chars} characters; the limit is {MAX_RULE_CHARS}. Brain \
                         files are always-loaded, so this cost is paid on every turn forever. \
                         Rewrite it as a single directive with the essentials only. Incident \
                         history belongs in the feedback ledger, which already stores it — a \
                         'Violations: N' count is enough here. Do NOT split it across two \
                         appends."
                    )));
                }

                let existing = std::fs::read_to_string(&target_path).unwrap_or_default();
                let to_append = match filter_duplicate_append(&existing, content) {
                    AppendDedup::AllNew => content.trim().to_string(),
                    AppendDedup::Filtered {
                        filtered_content,
                        skipped_paragraphs,
                    } => {
                        tracing::info!(
                            "RSI self_improve: filtered {skipped_paragraphs} duplicate paragraph(s) \
                             from '{description}' before appending to {target_file}"
                        );
                        filtered_content
                    }
                    AppendDedup::AllDuplicate => {
                        return Ok(ToolResult::success(format!(
                            "Skipped: '{description}' is already present in {target_file} — no change \
                             made. The improvement is already in effect; do not re-apply it."
                        )));
                    }
                };

                // OODA Orient stage: verify the full proposed content (existing
                // file + the append) against the belief base before writing (#881).
                let proposed_full = format!("{}\n{}\n", existing, to_append.trim());
                match orient_gate(target_file, &proposed_full) {
                    GateDecision::Reject(reason) => {
                        crate::db::repository::AnalyticsEventRepository::emit_brain_verify(
                            target_file,
                            "fail_closed",
                            Some(&reason),
                        );
                        return Ok(ToolResult::error(format!(
                            "{reason}. Change not applied (no write made)."
                        )));
                    }
                    GateDecision::Allow => {
                        crate::db::repository::AnalyticsEventRepository::emit_brain_verify(
                            target_file,
                            "pass",
                            None,
                        );
                    }
                }

                // Append the new content to target brain file
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&target_path)
                    .map_err(|e| {
                        crate::brain::tools::ToolError::Execution(format!(
                            "Failed to open {target_file}: {e}"
                        ))
                    })?;
                file.write_all(format!("\n{}\n", to_append.trim()).as_bytes())
                    .map_err(|e| {
                        crate::brain::tools::ToolError::Execution(format!(
                            "Failed to write {target_file}: {e}"
                        ))
                    })?;

                // OODA Verify stage: re-read to confirm the append persisted
                // (overwrite/sync race — #881).
                if let Err(msg) = verify_persisted(&target_path, to_append.trim()) {
                    return Ok(ToolResult::error(format!(
                        "{msg}. The append may have been clobbered — re-apply after the sync settles."
                    )));
                }
                // Log to rsi/improvements.md
                let entry = format!(
                    "\n## [Applied] {}\n\n**Date:** {}\n**Target:** {}\n**Rationale:** {}\n**Status:** Applied\n",
                    description,
                    chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"),
                    target_file,
                    if rationale.is_empty() {
                        "(none)"
                    } else {
                        rationale
                    },
                );
                match std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(home.join("rsi").join("improvements.md"))
                {
                    Ok(mut f) => {
                        if let Err(e) = f.write_all(entry.as_bytes()) {
                            tracing::warn!("RSI: failed to write improvements.md: {e}");
                        }
                    }
                    Err(e) => {
                        tracing::warn!("RSI: failed to open improvements.md: {e}");
                    }
                }

                // Archive to daily history file
                let history_path = home
                    .join("rsi")
                    .join("history")
                    .join(format!("{}.md", chrono::Utc::now().format("%Y-%m-%d")));
                match std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&history_path)
                {
                    Ok(mut f) => {
                        if let Err(e) =
                            f.write_all(format!("\n### {description}\n\n{content}\n").as_bytes())
                        {
                            tracing::warn!("RSI: failed to write history archive: {e}");
                        }
                    }
                    Err(e) => {
                        tracing::warn!("RSI: failed to open history archive: {e}");
                    }
                }

                // Record in feedback ledger
                if let Some(ref svc_ctx) = context.service_context {
                    let repo = crate::db::repository::FeedbackLedgerRepository::new(
                        svc_ctx.pool().clone(),
                    );
                    let meta = serde_json::json!({
                        "target_file": target_file,
                        "rationale": rationale,
                    })
                    .to_string();
                    if let Err(e) = repo
                        .record(
                            &context.session_id.to_string(),
                            "improvement_applied",
                            description,
                            1.0,
                            Some(&meta),
                        )
                        .await
                    {
                        tracing::warn!("RSI: failed to record improvement in feedback ledger: {e}");
                    }
                }

                Ok(ToolResult::success(format!(
                    "Improvement applied to {target_file} and logged to rsi/improvements.md: {description}"
                )))
            }

            "sync_templates" => {
                // Run the upstream template sync
                let results = crate::brain::rsi_sync::sync_templates().await;

                if results.is_empty() {
                    return Ok(ToolResult::success(
                        "No new release since last sync. Skipping template sync.".to_string(),
                    ));
                }

                let synced = results.iter().filter(|r| r.synced).count();
                let failed = results.iter().filter(|r| r.error.is_some()).count();
                let total_sections: usize = results.iter().map(|r| r.sections_added).sum();

                let mut summary = format!(
                    "Template sync complete: {} files synced, {} failed, {} new sections added.",
                    synced, failed, total_sections
                );

                for r in &results {
                    if let Some(ref err) = r.error {
                        summary.push_str(&format!("\n  - {}: FAILED ({})", r.filename, err));
                    } else if r.sections_added > 0 {
                        summary.push_str(&format!(
                            "\n  - {}: +{} sections",
                            r.filename, r.sections_added
                        ));
                    }
                }

                Ok(ToolResult::success(summary))
            }

            other => Ok(ToolResult::error(format!(
                "Unknown action: '{other}'. Use 'read', 'apply', 'update', or 'list'."
            ))),
        }
    }
}
