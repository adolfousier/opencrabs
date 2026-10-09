//! The compactor is told which skills are live, and a manifest that prunes one
//! is caught (#1960).
//!
//! The continuation document's `context-manifest` fence instructs the harness
//! which skills to keep (`active_skills`) and which to prune (`discard_skills`).
//! The compactor authored that fence from conversation memory alone: nothing in
//! its input said which skills the session actually held, and the budget guard
//! only checked that the fence EXISTED. A well-formed fence could therefore
//! revoke the #219 re-injection silently.
//!
//! Live specimen (2026-10-06), verbatim from a lane's compaction:
//!
//! ```context-manifest
//! active_skills:
//! - opencrabs-dev/harvest.md
//! discard_skills:
//! - opencrabs-dev
//! ```
//!
//! The parent body was pruned while its own auxiliary document was kept, so the
//! next `bash` call touching an `opencrabs-dev` path was refused by the skill
//! gate and the turn was spent reloading. Fence present, budget met, no warning.
//!
//! Three seams close it, and each is pinned here: the Manifest Rules now forbid
//! discarding an active skill, the live inventory is stamped into the request
//! (§10, ahead of the prose), and `warn_on_summary_invariants` reads the fence's
//! CONTENT against that inventory instead of only its shape.

use crate::brain::agent::service::AgentService;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;

/// A continuation document with a complete §0 and a §10 fence holding `manifest`.
fn doc_with_manifest(manifest: &str) -> String {
    format!(
        "## 0. IMMEDIATE TASK\n**Obligation status: OPEN**\n\
         CONTINUE THIS TASK: finish the retention guard.\n\n\
         ## 10. Context Manifest\n```context-manifest\n{manifest}```\n"
    )
}

const LIVE_SPECIMEN: &str =
    "active_skills:\n- opencrabs-dev/harvest.md\ndiscard_skills:\n- opencrabs-dev\n";

/// The live inventory is named in the request, as measured fact.
#[test]
fn the_stamp_names_every_live_skill() {
    let stamp = AgentService::compaction_active_skill_stamp(&[
        "github".to_string(),
        "opencrabs-dev".to_string(),
    ]);
    assert!(
        stamp.contains("opencrabs-dev") && stamp.contains("github"),
        "every live skill must be named, or the compactor cannot tell it from a \
         spent one: {stamp}"
    );
    assert!(
        stamp.contains("harness-measured"),
        "the stamp must say it is measured, not suggested, otherwise it reads as \
         another hint the summariser may weigh against its own judgement: {stamp}"
    );
    assert!(
        stamp.contains("`discard_skills`"),
        "the stamp must state the rule it exists to enforce: {stamp}"
    );
}

/// Zero marginal tokens for sessions that never touched a skill.
#[test]
fn the_stamp_is_silent_when_the_session_holds_no_skills() {
    assert!(
        AgentService::compaction_active_skill_stamp(&[]).is_empty(),
        "a session with no skills must not carry a skill block at all"
    );
}

/// The rule itself is in the static prompt, not only in the dynamic stamp.
#[test]
fn the_manifest_rules_forbid_discarding_an_active_skill() {
    let src = include_str!("../brain/agent/service/context.rs");
    assert!(
        src.contains("NEVER discard an ACTIVE skill (#1960)"),
        "the Manifest Rules block must carry the never-discard-active clause: an \
         empty inventory stamps nothing, and that is exactly when the rule is most \
         needed"
    );
    assert!(
        src.contains("{active_skill_stamp}"),
        "the live inventory must be interpolated into §10 of the summariser prompt"
    );
}

/// The 2026-10-06 specimen, caught.
#[test]
fn the_self_contradictory_specimen_is_caught() {
    let doc = doc_with_manifest(LIVE_SPECIMEN);
    let conflicts =
        AgentService::manifest_discard_conflicts(&doc, &["opencrabs-dev/harvest.md".to_string()]);
    assert_eq!(
        conflicts,
        vec!["opencrabs-dev".to_string()],
        "discarding the parent while its auxiliary document is active unloads both"
    );
}

/// A plain exact match is the simplest case.
#[test]
fn an_exact_active_name_discarded_is_caught() {
    let doc =
        doc_with_manifest("active_skills:\n- opencrabs-dev\ndiscard_skills:\n- opencrabs-dev\n");
    let conflicts = AgentService::manifest_discard_conflicts(&doc, &["opencrabs-dev".to_string()]);
    assert_eq!(conflicts, vec!["opencrabs-dev".to_string()]);
}

/// A consistent manifest is not flagged: the check is a contradiction detector,
/// not a blanket refusal to prune.
#[test]
fn a_consistent_manifest_conflicts_with_nothing() {
    let doc = doc_with_manifest("active_skills:\n- opencrabs-dev\ndiscard_skills:\n- grafana\n");
    let conflicts = AgentService::manifest_discard_conflicts(&doc, &["opencrabs-dev".to_string()]);
    assert!(
        conflicts.is_empty(),
        "pruning a skill the session does not hold is legitimate: {conflicts:?}"
    );
}

/// `dev` and `dev-tools` share a text prefix and nothing else.
#[test]
fn a_shared_prefix_is_not_a_parent() {
    let doc = doc_with_manifest("active_skills:\n- dev-tools\ndiscard_skills:\n- dev\n");
    let conflicts = AgentService::manifest_discard_conflicts(&doc, &["dev-tools".to_string()]);
    assert!(
        conflicts.is_empty(),
        "a name that merely starts the same way is a different skill: {conflicts:?}"
    );
}

/// Nothing to protect: an empty inventory must never fire.
#[test]
fn no_active_skills_means_nothing_to_conflict() {
    let doc = doc_with_manifest(LIVE_SPECIMEN);
    assert!(
        AgentService::manifest_discard_conflicts(&doc, &[]).is_empty(),
        "with no live skills the guard has no ground truth to compare against"
    );
}

/// A document with no fence at all is the presence check's problem, not this
/// one: the two guards must not double-report the same defect.
#[test]
fn a_document_without_a_fence_has_no_content_conflict() {
    let doc = "## 0. IMMEDIATE TASK\n**Obligation status: OPEN**\nCONTINUE THIS TASK: x\n";
    assert!(
        AgentService::manifest_discard_conflicts(doc, &["opencrabs-dev".to_string()]).is_empty()
    );
}

/// The guard wired into the real path: a self-contradictory fence, shipped
/// under budget, must surface a WARN naming the skill (#1960).
#[test]
fn summary_invariants_warn_when_the_manifest_prunes_an_active_skill() {
    let doc = doc_with_manifest(LIVE_SPECIMEN);
    let capture = EventCapture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    tracing::subscriber::with_default(subscriber, || {
        AgentService::enforce_summary_budget(
            doc.clone(),
            crate::brain::agent::service::request_budget::COMPACTION_SUMMARY_MAX_TOKENS as usize,
            &["opencrabs-dev/harvest.md".to_string()],
        )
    });

    let warns = capture.warns();
    assert!(
        warns.iter().any(|m| m.contains("#1960")
            && m.contains("opencrabs-dev")
            && m.contains("skill gate")),
        "the exact failure mode from the issue must be visible in the log, not \
         only in the next turn's refused tool; captured: {warns:?}"
    );
}

/// And the same path stays quiet when the manifest is consistent.
#[test]
fn summary_invariants_stay_silent_on_a_consistent_manifest() {
    let doc = doc_with_manifest("active_skills:\n- opencrabs-dev\ndiscard_skills:\n- grafana\n");
    let capture = EventCapture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    tracing::subscriber::with_default(subscriber, || {
        AgentService::enforce_summary_budget(
            doc.clone(),
            crate::brain::agent::service::request_budget::COMPACTION_SUMMARY_MAX_TOKENS as usize,
            &["opencrabs-dev".to_string()],
        )
    });

    assert!(
        capture.warns().is_empty(),
        "a complete, consistent document must not warn; captured: {:?}",
        capture.warns()
    );
}

// --- warning capture (house pattern, see compaction_summary_budget_test) ----

#[derive(Clone, Default)]
struct EventCapture {
    events: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
}

impl EventCapture {
    fn warns(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(level, _)| level == "WARN")
            .map(|(_, message)| message.clone())
            .collect()
    }
}

impl<S: tracing::Subscriber> Layer<S> for EventCapture {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        self.events.lock().unwrap().push((
            event.metadata().level().to_string(),
            visitor.message.unwrap_or_default(),
        ));
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        }
    }
}
