//! #147 — `output_full` persistence in the sub-agent status file.
//!
//! The four guarantees of the field swap, pinned so they cannot silently
//! regress: byte-exact roundtrip, serde-skip when absent, legacy-file
//! compat, and the honest push hint. The per-machinery tests live next to
//! the machinery they exercise (spawn.rs / work_status.rs);
//! this module is the one-stop index that runs them together.

use crate::brain::agent::service::work_status::{
    WorkState, WorkStatus, cleanup_stale, test_override,
};
use crate::brain::tools::subagent::spawn::completion_message;
use std::fs;
use std::time::Duration;

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "oc-output-full-test-{tag}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&dir).expect("temp dir");
    test_override::set(dir.clone());
    dir
}

fn drop_dir(dir: std::path::PathBuf) {
    test_override::clear();
    let _ = fs::remove_dir_all(dir);
}

/// (a) Roundtrip: a 5 KB report persisted via `mark_completed` reads back
/// byte-exact, and the JSON carries no `output_summary` field.
#[test]
fn roundtrip_five_kb_report_byte_exact_no_summary_field() {
    let dir = temp_dir("roundtrip");
    let report: String = "# REVIEW\n\n".to_string()
        + &"finding line with plenty of words to bulk it up.\n".repeat(120);
    assert!(report.chars().count() > 5000, "fixture must be ~5 KB");

    let mut s = WorkStatus::new_agent("ofx-1", "review", "sess-x", "review", None).unwrap();
    s.mark_completed(report.clone()).unwrap();
    let finish = s.finish.as_ref().expect("finish stamped");
    assert_eq!(finish.output_full.as_deref(), Some(report.as_str()));

    let parsed: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("ofx-1.json")).unwrap()).unwrap();
    assert_eq!(
        parsed["finish"]["output_full"].as_str(),
        Some(report.as_str())
    );
    assert!(parsed["finish"].get("output_summary").is_none());
    drop_dir(dir);
}

/// (b) Serde skip: absent `output_full` stays absent — no `null`
/// placeholders polluting the JSON of files that never completed.
#[test]
fn absent_field_stays_absent_no_nulls() {
    let dir = temp_dir("skip");
    let _s = WorkStatus::new_agent("ofx-2", "idle", "sess-x2", "nothing", None).unwrap();
    let raw = fs::read_to_string(dir.join("ofx-2.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(parsed.get("finish").is_none());
    // Precise guarantee: output_full carries skip_serializing_if, so it must
    // not appear at all (an output_full: null placeholder would violate it).
    // Other Option fields may legitimately serialize as null.
    assert!(
        !raw.contains("output_full"),
        "no output_full placeholder in fresh file"
    );
    drop_dir(dir);
}

/// (c) Legacy compat: a pre-#147 file carrying `output_summary`
/// deserializes cleanly (unknown field ignored), no backfill.
#[test]
fn legacy_summary_file_deserializes_cleanly() {
    let dir = temp_dir("legacy");
    let legacy = serde_json::json!({
        "id": "ofx-3",
        "kind": "agent",
        "session_id": "sess-x3",
        "label": "old",
        "task": "old task",
        "spawned_at": "2026-08-28T09:00:00+00:00",
        "state": "Completed",
        "finish": {
            "completed_at": "2026-08-28T09:30:00+00:00",
            "output_summary": "all done"
        }
    });
    fs::write(
        dir.join("ofx-3.json"),
        serde_json::to_string_pretty(&legacy).unwrap(),
    )
    .unwrap();
    let s = WorkStatus::read("ofx-3").expect("legacy file parses");
    assert_eq!(s.state, WorkState::Completed);
    let finish = s.finish.as_ref().expect("finish present");
    assert_eq!(finish.output_full, None, "no backfill of the old stub");
    // And the cleanup still ages it out by completed_at.
    assert_eq!(cleanup_ages_it(&dir), 1);
    drop_dir(dir);
}

fn cleanup_ages_it(dir: &std::path::Path) -> usize {
    let old_ts = chrono::Utc::now()
        .checked_sub_signed(chrono::Duration::days(8))
        .unwrap()
        .to_rfc3339();
    let mut parsed: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("ofx-3.json")).unwrap()).unwrap();
    parsed["finish"]["completed_at"] = serde_json::json!(old_ts);
    fs::write(
        dir.join("ofx-3.json"),
        serde_json::to_string_pretty(&parsed).unwrap(),
    )
    .unwrap();
    cleanup_stale(Duration::from_secs(7 * 86400))
        .map(|(_, removed)| removed)
        .unwrap_or(0)
}

/// (d) Honest hint: a >4000-char pushed output names the persisted file and
/// `output_full`, never the RAM-resident `wait_agent`; the path is the real
/// `status_path`, not a bare id.
#[test]
fn hint_names_persisted_file_with_real_path() {
    let long: String = std::iter::repeat_n('x', 5000)
        .chain("THE-CONCLUSION".chars())
        .collect();
    let msg = completion_message("big", "ofx-hint", Ok(&long));
    assert!(!msg.context_text.contains("wait_agent"));
    assert!(msg.context_text.contains("output_full"));
    let expected = crate::brain::agent::service::work_status::status_path("ofx-hint");
    assert!(msg.context_text.contains(&expected.display().to_string()));
}
