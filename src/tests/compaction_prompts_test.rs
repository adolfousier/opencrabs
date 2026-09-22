//! Tests for `agent::service::compaction_prompts`.
//!
//! Regression context (2026-05-29): a Telegram user (leshchenko1979)
//! forwarded a post-compaction one-liner — the model dropping into
//! Russian мат when expressing frustration after recovering context —
//! to friends, calling it a "Skynet" moment. Earlier we had silenced
//! all post-compaction narration via commit fb325fb5; this test
//! locks in that the fun variant is the DEFAULT (so those delight
//! moments keep happening) and that the silent variant is reachable
//! when `[agent] silent_compaction = true`.
//!
//! These tests are deliberately string-sentinel based, not full
//! equality. The exact wording will drift; what must not drift is:
//!
//! - Default (silent=false) keeps the explicit "POST-COMPACTION
//!   PROTOCOL" header and at least one invitation to acknowledge.
//! - Silent (silent=true) keeps the explicit "Silently" directive
//!   and excludes the fun acknowledgement invitation.
//! - The auto-approve tail is appended in both modes when
//!   auto_approve=false.

use crate::brain::agent::service::compaction_prompts::{
    CompactionKind, PlanRecovery, append_skill_stamp, build_continuation,
};

const APPROVAL_TAIL: &str = "Tool approval is REQUIRED";
const SESSION_RECOVERY: &str = "SESSION RECOVERY";

#[test]
fn fun_regular_keeps_post_compaction_protocol_header() {
    let body = build_continuation(CompactionKind::Regular, false, true, PlanRecovery::Active);
    assert!(
        body.contains("POST-COMPACTION PROTOCOL"),
        "fun regular must include the numbered protocol header so the \
         model picks up the task; got: {body}"
    );
    assert!(
        !body.contains("Silently continue"),
        "fun variant must not carry the silent directive: {body}"
    );
}

#[test]
fn silent_regular_uses_silent_directive() {
    let body = build_continuation(CompactionKind::Regular, true, true, PlanRecovery::Active);
    assert!(
        body.contains("Silently continue"),
        "silent regular must explicitly tell the model to continue silently: {body}"
    );
    assert!(
        !body.contains("POST-COMPACTION PROTOCOL"),
        "silent variant must drop the verbose protocol header: {body}"
    );
}

#[test]
fn fun_emergency_invites_fun_cheeky_remark() {
    // The emergency path is where the most-shared one-liners came
    // from. The fun variant MUST carry an explicit invitation, not
    // just allow it implicitly — otherwise the model defaults to
    // silent recovery and the personality moment never fires.
    let body = build_continuation(CompactionKind::Emergency, false, true, PlanRecovery::Active);
    assert!(
        body.contains("fun/cheeky ROAST"),
        "fun emergency must explicitly invite a fun/cheeky roast: {body}"
    );
}

#[test]
fn silent_emergency_suppresses_acknowledgement() {
    let body = build_continuation(CompactionKind::Emergency, true, true, PlanRecovery::Active);
    assert!(
        body.contains("Silently resume"),
        "silent emergency must direct silent resumption: {body}"
    );
    assert!(
        !body.contains("fun/cheeky remark"),
        "silent variant must not invite a fun remark: {body}"
    );
}

#[test]
fn fun_post_tool_keeps_cursing_allowed_explicit() {
    // The post-tool prompt historically carried "cursing allowed" as
    // an explicit license. That's the line that produces the kind
    // of in-character output users have called out. If we ever
    // trim it we break the documented feature.
    let body = build_continuation(CompactionKind::PostTool, false, true, PlanRecovery::Active);
    assert!(
        body.contains("cursing allowed"),
        "fun post-tool must keep the explicit cursing-allowed license: {body}"
    );
    assert!(body.contains("IMMEDIATE TASK"));
}

#[test]
fn silent_post_tool_drops_cursing_invitation() {
    let body = build_continuation(CompactionKind::PostTool, true, true, PlanRecovery::Active);
    assert!(!body.contains("cursing allowed"));
    assert!(body.contains("Silently continue"));
}

#[test]
fn mid_loop_variants_diverge_on_silent_flag() {
    let fun = build_continuation(CompactionKind::MidLoop, false, true, PlanRecovery::Active);
    let silent = build_continuation(CompactionKind::MidLoop, true, true, PlanRecovery::Active);
    assert_ne!(fun, silent, "the two modes must produce different prompts");
    assert!(fun.contains("POST-COMPACTION PROTOCOL"));
    assert!(silent.contains("Silently continue"));
}

#[test]
fn approval_tail_appended_when_auto_approve_disabled_in_both_modes() {
    for kind in [
        CompactionKind::Regular,
        CompactionKind::MidLoop,
        CompactionKind::Emergency,
        CompactionKind::PostTool,
    ] {
        let fun = build_continuation(kind, false, false, PlanRecovery::Active);
        let silent = build_continuation(kind, true, false, PlanRecovery::Active);
        assert!(
            fun.contains(APPROVAL_TAIL),
            "fun {kind:?} must append the approval reminder when auto_approve=false: {fun}"
        );
        assert!(
            silent.contains(APPROVAL_TAIL),
            "silent {kind:?} must append the approval reminder when auto_approve=false: {silent}"
        );
    }
}

#[test]
fn approval_tail_omitted_when_auto_approve_enabled() {
    for kind in [
        CompactionKind::Regular,
        CompactionKind::MidLoop,
        CompactionKind::Emergency,
        CompactionKind::PostTool,
    ] {
        let fun = build_continuation(kind, false, true, PlanRecovery::Active);
        let silent = build_continuation(kind, true, true, PlanRecovery::Active);
        assert!(
            !fun.contains(APPROVAL_TAIL),
            "fun {kind:?} must NOT carry the approval tail when auto_approve=true: {fun}"
        );
        assert!(
            !silent.contains(APPROVAL_TAIL),
            "silent {kind:?} must NOT carry the approval tail when auto_approve=true: {silent}"
        );
    }
}

#[test]
fn default_agent_config_is_fun_mode() {
    // The `silent_compaction` flag is what selects between modes;
    // verify the config default keeps fun mode active so a fresh
    // install gets the personality moments out of the box.
    let cfg = crate::config::AgentConfig::default();
    assert!(
        !cfg.silent_compaction,
        "AgentConfig::default() must keep silent_compaction=false so fun \
         post-compaction narration is the out-of-the-box behaviour"
    );
}

#[test]
fn session_recovery_hint_present_in_all_variants() {
    // After compaction the agent must check for an active plan and,
    // if coding, load CODE.md. This hint is appended to ALL variants
    // (fun + silent, all 5 kinds) so it never gets lost.
    for kind in [
        CompactionKind::Regular,
        CompactionKind::MidLoop,
        CompactionKind::Emergency,
        CompactionKind::PostTool,
        CompactionKind::Manual,
    ] {
        for silent in [false, true] {
            let body = build_continuation(kind, silent, true, PlanRecovery::Active);
            assert!(
                body.contains(SESSION_RECOVERY),
                "{kind:?} silent={silent} must include the SESSION RECOVERY hint: {body}"
            );
            assert!(
                body.contains("plan") && body.contains("start"),
                "{kind:?} silent={silent} must tell the agent to call plan with start: {body}"
            );
            assert!(
                body.contains("CODE.md"),
                "{kind:?} silent={silent} must mention CODE.md for coding sessions: {body}"
            );
        }
    }
}

#[test]
fn manual_compaction_is_brief() {
    // Manual /compact uses a short sentence, not the full POST-COMPACTION
    // PROTOCOL with numbered steps. The user explicitly triggered it.
    for silent in [false, true] {
        let body = build_continuation(CompactionKind::Manual, silent, true, PlanRecovery::Active);
        assert!(
            !body.contains("POST-COMPACTION PROTOCOL"),
            "Manual compaction must NOT use the full protocol: {body}"
        );
        assert!(
            !body.contains("session_search"),
            "Manual compaction must NOT instruct session_search (brief only): {body}"
        );
        assert!(
            body.contains("IMMEDIATE TASK"),
            "Manual compaction must still reference the IMMEDIATE TASK: {body}"
        );
    }
}

// ── #125: advisory skill-inventory stamp ────────────────────────────────────
//
// The stamp must: (1) carry the sorted skill list with the advisory
// "consider, reload only those" framing, (2) be absent entirely when no
// skills were active (zero marginal tokens), (3) ride EVERY compaction
// kind in both fun and silent variants. String sentinels, same policy
// as the rest of this file: exact wording may drift, these invariants
// must not.

const STAMP_HEADER: &str = "SKILLS LOADED PRE-COMPACTION:";
const STAMP_ADVISORY: &str = "reload only those that are";

#[test]
fn skill_stamp_lists_active_skills_with_advisory_framing() {
    let body = build_continuation(CompactionKind::Regular, false, true, PlanRecovery::Active);
    let stamped = append_skill_stamp(body, &["opencrabs-dev".to_string(), "grafana".to_string()]);
    assert!(
        stamped.contains(STAMP_HEADER),
        "stamp must carry the inventory header: {stamped}"
    );
    assert!(
        stamped.contains("opencrabs-dev") && stamped.contains("grafana"),
        "stamp must list every active skill: {stamped}"
    );
    assert!(
        stamped.contains(STAMP_ADVISORY),
        "stamp must be advisory (consider, reload selectively) not prescriptive: {stamped}"
    );
}

#[test]
fn skill_stamp_is_silent_when_no_skills_active() {
    for kind in [
        CompactionKind::Regular,
        CompactionKind::MidLoop,
        CompactionKind::Emergency,
        CompactionKind::PostTool,
        CompactionKind::Manual,
    ] {
        for silent in [false, true] {
            let body = build_continuation(kind, silent, true, PlanRecovery::NoPlan);
            let stamped = append_skill_stamp(body, &[]);
            assert!(
                !stamped.contains(STAMP_HEADER),
                "{kind:?} silent={silent}: no skills → no stamp, zero marginal tokens: {stamped}"
            );
        }
    }
}

#[test]
fn skill_stamp_rides_all_kinds_and_variants() {
    let skills = vec!["opencrabs-dev".to_string()];
    for kind in [
        CompactionKind::Regular,
        CompactionKind::MidLoop,
        CompactionKind::Emergency,
        CompactionKind::PostTool,
        CompactionKind::Manual,
    ] {
        for silent in [false, true] {
            let body = build_continuation(kind, silent, true, PlanRecovery::Active);
            let stamped = append_skill_stamp(body, &skills);
            assert!(
                stamped.contains(STAMP_HEADER) && stamped.contains(STAMP_ADVISORY),
                "{kind:?} silent={silent}: stamp must ride every kind/variant: {stamped}"
            );
        }
    }
}

#[test]
fn skill_stamp_sorts_names_for_determinism() {
    let body = build_continuation(CompactionKind::Regular, false, true, PlanRecovery::NoPlan);
    let stamped = append_skill_stamp(
        body,
        &["zeta".to_string(), "alpha".to_string(), "mid".to_string()],
    );
    let header_pos = stamped.find(STAMP_HEADER).expect("stamp present");
    let list = &stamped[header_pos..];
    let alpha = list.find("alpha").expect("alpha present");
    let mid = list.find("mid").expect("mid present");
    let zeta = list.find("zeta").expect("zeta present");
    assert!(
        alpha < mid && mid < zeta,
        "stamp list must be sorted for deterministic rendering: {list}"
    );
}

/// The roast is a per-kind obligation inside `fun_body`, and that is
/// exactly how it vanished. `d3c239c1e` (2026-05-30, "restore fun
/// post-compaction narration") shipped the fun variant with the
/// acknowledgement line in only two of five arms, Emergency and
/// PostTool, leaving `Regular` and `MidLoop` with none. Those two are
/// the dominant auto paths (11 of 18 continuations across the logs
/// surviving on 2026-09-22), so the delight feature the restore commit
/// was meant to bring back stayed dark for 115 days.
///
/// `compaction_single_continuation_path_test.rs` guards the CALL SITE
/// (the loop reaches the builder exactly once). Nothing guarded the
/// ARMS, so a missing arm was invisible to every test in this file.
/// This pins the property the sentinel tests cannot.
#[test]
fn every_fun_arm_except_manual_invites_a_roast_by_name() {
    for kind in [
        CompactionKind::Regular,
        CompactionKind::MidLoop,
        CompactionKind::Emergency,
        CompactionKind::PostTool,
    ] {
        let body = build_continuation(kind, false, true, PlanRecovery::Active);
        assert!(
            body.to_lowercase().contains("roast"),
            "fun {kind:?} must name the ROAST explicitly. The model follows \
             the loudest directive and drops a parenthetical aside, which is \
             how a 'fun/cheeky remark' lost to an 'IMMEDIATELY continue ... \
             NOT optional' sibling for 115 days: {body}"
        );
    }
}

/// Manual is the deliberate exception: the user typed `/compact`, so they
/// already saw the confirmation and acknowledging it back to them is
/// noise. Pinned as an exception rather than an oversight, so the next
/// contributor does not "fix" it back in.
#[test]
fn manual_still_forbids_the_acknowledgement() {
    let body = build_continuation(CompactionKind::Manual, false, true, PlanRecovery::Active);
    assert!(
        body.contains("Do NOT acknowledge the compaction"),
        "manual must stay acknowledgement-free, the user asked for it: {body}"
    );
    assert!(
        !body.to_lowercase().contains("roast"),
        "manual must not invite a roast: {body}"
    );
}

/// The silent variant must stay free of the roast invitation, or
/// `silent_compaction = true` leaks personality into corporate
/// deployments, which is the only reason the flag exists.
#[test]
fn silent_arms_never_invite_a_roast() {
    for kind in [
        CompactionKind::Regular,
        CompactionKind::MidLoop,
        CompactionKind::Emergency,
        CompactionKind::PostTool,
        CompactionKind::Manual,
    ] {
        let body = build_continuation(kind, true, true, PlanRecovery::Active);
        assert!(
            !body.to_lowercase().contains("roast"),
            "silent {kind:?} must not invite a roast: {body}"
        );
    }
}
