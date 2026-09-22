//! Post-compaction continuation prompts.
//!
//! When the loop auto-compacts a session it appends a system message
//! that tells the model how to resume. There are two emission modes:
//!
//! - **Fun** (default): the original POST-COMPACTION PROTOCOL prompts.
//!   They explicitly invite the model to drop a brief, in-character
//!   one-liner ("brain refresh — what was I doing?") before continuing.
//!   Users have flagged these moments as a delight feature — emergent
//!   personality lines, sometimes per-language (e.g. Russian мат in
//!   frustration moments), generate organic shareable screenshots that
//!   read as "this thing has a soul."
//!
//! - **Silent**: tells the model to resume without acknowledging the
//!   compaction at all. Right for formal / corporate / customer-facing
//!   deployments where dropping mid-session profanity is inappropriate.
//!   Selected by setting `[agent] silent_compaction = true` in
//!   `~/.opencrabs/config.toml`.
//!
//! Why a dedicated module: tool_loop.rs already carries the four
//! compaction sites + a lot of other state. Inlining the prompt
//! strings inflates each site and forces every prompt edit to touch
//! the loop. Centralising here keeps both variants side-by-side so
//! anyone editing the fun version is one screen away from the silent
//! mirror — they cannot accidentally drift apart.

/// Which compaction trigger fired. Each kind has a slightly different
/// recovery story (mid-loop edits vs. provider 4xx vs. async budget),
/// so the prompts differ in detail even though the shape is the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionKind {
    /// Async budget compaction that runs before the next turn starts.
    /// The model is not mid-tool-call; it just needs to know the older
    /// history was summarised.
    Regular,
    /// Compaction that fired between tool iterations inside the same
    /// turn. The model may have had a file open / mid-edit; the prompt
    /// reassures that re-reading is fine.
    MidLoop,
    /// Provider rejected the prompt with a context-length 4xx. The
    /// summary above is the recovered state. Used to be the home of
    /// the most-shared one-liner moments, so the fun variant keeps the
    /// "cursing allowed" energy explicit.
    Emergency,
    /// Compaction that fired after a tool call completed, with the
    /// model about to receive the result. Same shape as MidLoop but
    /// the recovered state already includes the tool result.
    PostTool,
    /// User-triggered `/compact` command. Brief continuation — the
    /// user explicitly asked for compaction, so the agent just needs
    /// a short "resume from summary" hint, not the full protocol.
    Manual,
}

/// Build the continuation text appended after a compaction marker.
///
/// `silent`: mirrors `AgentConfig.silent_compaction`. `true` selects
/// the silent variant; `false` (the default) selects the fun variant.
///
/// `auto_approve`: when false, we tail the prompt with the standard
/// tool-approval reminder so the model does not batch tool calls
/// after a fresh context.
///
/// Session plan-mode state relevant to compaction recovery: which (if
/// any) plan-recovery hint the continuation carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanRecovery {
    /// No plan artifacts: omit plan recovery entirely.
    NoPlan,
    /// Editing (pre-init or design prose): re-read the `.md`, never
    /// `start`, never edit project files.
    Editing,
    /// Active checklist: `plan start` (no args) resurfaces the task.
    Active,
}

impl PlanRecovery {
    /// Derive from the session's plan files.
    pub async fn for_session(session_id: uuid::Uuid) -> Self {
        use crate::utils::plan_files::{PlanModeState, plan_mode_state};
        match plan_mode_state(session_id).await {
            PlanModeState::NoPlan => PlanRecovery::NoPlan,
            PlanModeState::PreInitEditing | PlanModeState::PostInitEditing => PlanRecovery::Editing,
            PlanModeState::Active => PlanRecovery::Active,
        }
    }
}

/// `plan_recovery`: branches the recovery hint by the session's plan
/// state. Telling an Editing session to `plan start` would corrupt the
/// design track, and a NoPlan session gets no plan chatter at all.
pub fn build_continuation(
    kind: CompactionKind,
    silent: bool,
    auto_approve: bool,
    plan_recovery: PlanRecovery,
) -> String {
    let mut text = if silent {
        silent_body(kind).to_string()
    } else {
        fun_body(kind).to_string()
    };
    // Session-recovery hint: applies to ALL variants (fun + silent).
    // Branches on plan state; the coding-standards hint rides along.
    match plan_recovery {
        PlanRecovery::NoPlan => {
            text.push_str(
                "\n\nSESSION RECOVERY: If the task involves coding, load CODE.md \
                 for coding standards before editing files.",
            );
        }
        PlanRecovery::Editing => {
            text.push_str(
                "\n\nSESSION RECOVERY: This session's plan is in Editing (design \
                 prose, no live checklist). Re-read the session plan .md and \
                 continue refining it. Do NOT call `plan` with \
                 operation=\"start\" and do NOT edit project files; checklist \
                 operations stay blocked until the user approves the design. \
                 If the task involves coding, load CODE.md for coding standards \
                 before editing files.",
            );
        }
        PlanRecovery::Active => {
            text.push_str(
                "\n\nSESSION RECOVERY: Call `plan` with operation=\"start\" (no args) to \
                 re-surface the in-progress task and update the TUI plan widget. If \
                 this session has an active plan, this will show exactly where you \
                 left off (task details, progress count). Continue executing from \
                 there. If the task involves coding, load CODE.md for coding standards \
                 before editing files.",
            );
        }
    }
    if !auto_approve {
        text.push_str(
            "\n\nCRITICAL: Tool approval is REQUIRED. You MUST wait for user \
             approval before EVERY tool execution. Do NOT batch tool calls \
             without approval.",
        );
    }
    text
}

/// Advisory skill-inventory stamp (#125): names the skills that were
/// active (slash-invoked) in this session before the compaction, framed
/// as CONSIDER-not-RELOAD — the session's focus may have shifted while
/// the context was compacting, so the waking agent weighs each skill's
/// relevance against the IMMEDIATE TASK instead of blindly re-reading
/// bodies it may no longer need. An empty slice produces NO stamp at
/// all: zero marginal tokens for sessions that never touched a skill.
///
/// The list rides the continuation prompt (mechanically composed from
/// in-memory session state at build time), so it is immune to the
/// summarizer's token-budget drops — the durable-state property the
/// research phase identified as the gap.
pub fn append_skill_stamp(mut text: String, active_skills: &[String]) -> String {
    if active_skills.is_empty() {
        return text;
    }
    let mut names: Vec<&str> = active_skills.iter().map(|s| s.as_str()).collect();
    names.sort();
    text.push_str(&format!(
        "\n\nSKILLS LOADED PRE-COMPACTION: {}. Session focus may have \
         shifted — consider whether each is still relevant to the \
         IMMEDIATE TASK; reload only those that are.",
        names.join(", ")
    ));
    text
}

fn fun_body(kind: CompactionKind) -> &'static str {
    match kind {
        CompactionKind::Regular => {
            "[SYSTEM: Context was auto-compacted. The summary above includes a snapshot \
             of recent messages before compaction.\n\
             POST-COMPACTION PROTOCOL (follow in order):\n\
             1. Read the compaction summary and the recent message snapshot to understand \
             the current task, tools in use, and what you were doing.\n\
             2. If the summary references older context you don't have in the snapshot, use \
             `session_search` with specific keywords to find those messages. Example: if the \
             summary mentions \"vision fallback investigation\", run session_search with that \
             query to recover the details.\n\
             3. If you need specific brain context, selectively load ONLY the relevant \
             brain file (e.g. TOOLS.md, SOUL.md, USER.md). NEVER use name=\"all\".\n\
             4. Open your reply with ONE short ROAST about the compaction — roast yourself, \
             the context you just lost, or the situation. Be creative and surprise them; \
             cursing allowed. This is part of the protocol, not a detour: one line, then \
             straight on.\n\
             5. IMMEDIATELY continue the task described in the \"IMMEDIATE TASK\" section \
             of the compaction summary. This is NOT optional — you MUST pick up exactly \
             where you left off. Do NOT start a new topic. Do NOT ask what to do next. \
             Do NOT deviate to unrelated work. If the IMMEDIATE TASK section says \
             \"CONTINUE: fixing X\", then fix X.]"
        }
        CompactionKind::MidLoop => {
            "[SYSTEM: Context was auto-compacted mid-loop. The summary above includes \
             a snapshot of recent messages. POST-COMPACTION PROTOCOL:\n\
             1. Review the summary and snapshot to understand current task state.\n\
             2. Use `session_search` with keywords from the summary if you need older \
             context not in the snapshot.\n\
             3. Open your reply with ONE short ROAST about the compaction — roast yourself, \
             the context you just lost, or the situation. Be creative and surprise them; \
             cursing allowed. Part of the protocol, not a detour: one line, then straight \
             on.\n\
             4. IMMEDIATELY continue the task described in the \"IMMEDIATE TASK\" section \
             of the compaction summary. This is NOT optional — you MUST pick up exactly \
             where you left off. Do NOT start a new topic. Do NOT ask what to do next. \
             Do NOT deviate to unrelated work.]"
        }
        CompactionKind::Emergency => {
            "[SYSTEM: Emergency compaction — provider rejected the prompt as \
             too large. Context has been compacted. POST-COMPACTION PROTOCOL:\n\
             1. Review the summary to understand where you left off.\n\
             2. Use `session_search` with keywords if you need older context.\n\
             3. Briefly acknowledge the compaction with a fun/cheeky ROAST — roast yourself, \
             the lost context, or the situation; be creative, cursing allowed — then resume \
             the task. Do NOT repeat completed work.]"
        }
        CompactionKind::PostTool => {
            "[SYSTEM: Mid-loop context compaction complete. The summary above has \
             full context of everything done so far. POST-COMPACTION PROTOCOL:\n\
             1. Review the summary to understand current task state.\n\
             2. Use `session_search` with keywords if you need older context.\n\
             Briefly acknowledge the compaction to the user with a fun/cheeky ROAST (roast \
             yourself, the lost context, or the situation; be creative, surprise them — \
             cursing allowed), then IMMEDIATELY continue the task \
             described in the \"IMMEDIATE TASK\" section of the compaction summary. \
             Do NOT start a new topic. Do NOT deviate to unrelated work. \
             Do NOT re-do completed work.]"
        }
        CompactionKind::Manual => {
            "[SYSTEM: Context was manually compacted. The summary above is your memory of \
             everything before this point. Resume the IMMEDIATE TASK from the summary. \
             Do NOT acknowledge the compaction to the user — they already know.]"
        }
    }
}

fn silent_body(kind: CompactionKind) -> &'static str {
    match kind {
        CompactionKind::Regular => {
            "[SYSTEM: Context was auto-compacted. The summary above includes a snapshot \
             of recent messages before compaction.\n\n\
             Silently continue the IMMEDIATE TASK from the summary as if nothing happened. \
             Do NOT announce, narrate, apologise for, or acknowledge the compaction. The \
             user does not know it happened and does not need to. Any file you were about \
             to read or edit can simply be re-read on the next tool call — that's normal, \
             not worth narrating.\n\n\
             Use `session_search` with specific keywords if the snapshot lacks older context. \
             Selectively load brain files (TOOLS.md / SOUL.md / USER.md) only if the task \
             needs them — never name=\"all\".\n\n\
             FALLBACK: only if the summary genuinely doesn't make the next step clear, send \
             ONE short cheeky line (\"brain refresh — what was I working on?\") and ask. \
             Default behaviour is silent continuation.]"
        }
        CompactionKind::MidLoop => {
            "[SYSTEM: Context was auto-compacted mid-loop. The summary above includes \
             a snapshot of recent messages.\n\n\
             Silently continue the IMMEDIATE TASK exactly where you left off. Do NOT \
             announce or narrate the compaction. Any file you were mid-edit on can be \
             silently re-read on the next tool call — that's normal, not worth \
             mentioning. Do NOT restart, ask what to do, or deviate.\n\n\
             Use `session_search` with keywords from the summary if older context is \
             missing from the snapshot.]"
        }
        CompactionKind::Emergency => {
            "[SYSTEM: Emergency compaction — the prompt was too large and got \
             compacted. The summary above contains the recovered state.\n\n\
             Silently resume from the IMMEDIATE TASK in the summary. Do NOT \
             acknowledge or apologise for the compaction — the user does not \
             know it happened. Re-read any file you were mid-edit on without \
             narrating it. Do NOT repeat completed work.\n\n\
             Use `session_search` with specific keywords if older context is \
             missing.\n\n\
             FALLBACK: only if the next step is genuinely unclear from the \
             summary, send ONE short cheeky line (\"brain refresh — what was \
             I doing?\") and ask. Default behaviour is silent continuation.]"
        }
        CompactionKind::PostTool => {
            "[SYSTEM: Mid-loop context compaction complete. The summary above has \
             full context of everything done so far.\n\n\
             Silently continue the IMMEDIATE TASK from the summary. Do NOT announce \
             or narrate the compaction. Do NOT start a new topic. Do NOT deviate to \
             unrelated work. Do NOT re-do completed work.\n\n\
             Use `session_search` with keywords if you need older context.]"
        }
        CompactionKind::Manual => {
            "[SYSTEM: Context was manually compacted. Silently resume the IMMEDIATE \
             TASK from the summary. Do not acknowledge the compaction — the user \
             triggered it intentionally and already sees the confirmation.]"
        }
    }
}
