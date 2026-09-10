//! #124: tmp photo/voice pickup must be thread-scoped so one topic's
//! attachment can never leak into another topic's session context.
//!
//! Covers: the thread gate in `find_recent_tmp_file` /
//! `find_all_recent_tmp_files` (new `t{thread}` names match only their
//! owning topic; legacy bare names only the scope-0 scanner that saved
//! them), and consume-once pickup for photos.

use crate::channels::telegram::media::{find_all_recent_tmp_files, find_recent_tmp_file};
use crate::config::profile::with_home_override;

fn seed(name: &str) {
    let tmp = crate::config::opencrabs_home().join("tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join(name), b"x").unwrap();
}

fn clean_tmp() {
    let tmp = crate::config::opencrabs_home().join("tmp");
    if let Ok(entries) = std::fs::read_dir(&tmp) {
        for e in entries.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("photo-") || n.starts_with("voice-") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

#[test]
fn photo_scan_matches_only_owning_topic() {
    with_home_override(std::env::temp_dir().join("oc-124-owning"), || {
        clean_tmp();
        // Timestamp must be computed at runtime: the 300s recency window
        // compares against the real clock, so a hardcoded ts expires and
        // turns this test into a time bomb (seen 2026-09-08 20:45Z).
        let ts = chrono::Utc::now().timestamp() - 10;
        seed(&format!("photo--1003-t7-{ts}.jpg"));
        // Owning topic sees it.
        let own = find_all_recent_tmp_files(-1003, "photo", 300, 7);
        assert_eq!(own.len(), 1, "owning topic must pick up its photo");
        // A sibling topic must NOT see it — the leak being fixed.
        let other = find_all_recent_tmp_files(-1003, "photo", 300, 9);
        assert!(
            other.is_empty(),
            "sibling topic picked up another topic's photo (#124 leak)"
        );
        clean_tmp();
    });
}

#[test]
fn legacy_names_match_only_scope_zero() {
    with_home_override(std::env::temp_dir().join("oc-124-legacy"), || {
        clean_tmp();
        let ts = chrono::Utc::now().timestamp() - 10;
        seed(&format!("photo--1003-{ts}.jpg"));
        // Backwards-compat (acceptance 3): the General/DM scope (0) that
        // saved it still picks it up...
        let zero = find_all_recent_tmp_files(-1003, "photo", 300, 0);
        assert_eq!(
            zero.len(),
            1,
            "legacy file must stay visible to its saving scope"
        );
        // ...while any real topic ignores it (can't attribute ownership).
        let topic = find_all_recent_tmp_files(-1003, "photo", 300, 5);
        assert!(
            topic.is_empty(),
            "legacy file must not leak into a topic scan"
        );
        clean_tmp();
    });
}

#[test]
fn general_topic_and_dm_scopes_still_work() {
    with_home_override(std::env::temp_dir().join("oc-124-general"), || {
        clean_tmp();
        // New-format General/DM save: thread embedded as 0.
        let ts = chrono::Utc::now().timestamp() - 10;
        seed(&format!("photo--1003-t0-{ts}.jpg"));
        let dm = find_all_recent_tmp_files(-1003, "photo", 300, 0);
        assert_eq!(dm.len(), 1, "General/DM scope must see its own t0 saves");
        clean_tmp();
    });
}

#[test]
fn recent_photo_single_helper_respects_thread_gate() {
    with_home_override(std::env::temp_dir().join("oc-124-single"), || {
        clean_tmp();
        let ts = chrono::Utc::now().timestamp() - 10;
        seed(&format!("photo--1003-t4-{ts}.jpg"));
        assert!(find_recent_tmp_file(-1003, "photo", 300, 4).is_some());
        assert!(find_recent_tmp_file(-1003, "photo", 300, 6).is_none());
        clean_tmp();
    });
}

#[test]
fn age_window_still_applies_inside_owning_topic() {
    with_home_override(std::env::temp_dir().join("oc-124-age"), || {
        clean_tmp();
        // Ancient relative to now: stale, must not match even for owner.
        let ts = chrono::Utc::now().timestamp() - 1_000_000_000;
        seed(&format!("photo--1003-t2-{ts}.jpg"));
        let r = find_all_recent_tmp_files(-1003, "photo", 300, 2);
        assert!(r.is_empty(), "stale file must not match even for its owner");
        clean_tmp();
    });
}
