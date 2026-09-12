//! A session is linked to its project on the first turn, not only when someone
//! runs `/cd`.
//!
//! Sessions were titled automatically on their first turn but never linked to
//! a project at that moment. The only path that created the link was the `/cd`
//! handler, so a session carried a correct `working_directory` and a
//! `project_id` of NULL unless the user happened to change directory by hand,
//! and every per-project view and cost rollup under-reported by however many
//! sessions that was (#1445).

use crate::db::Database;
use crate::db::models::Project;
use crate::services::project_match::{DirectoryIdentity, match_by_directory};
use crate::services::{ProjectService, ServiceContext, SessionService};

async fn services() -> (ProjectService, SessionService) {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let context = ServiceContext::new(db.pool().clone());
    (
        ProjectService::new(context.clone()),
        SessionService::new(context),
    )
}

fn project_named(name: &str) -> Project {
    Project::new(name.to_string(), None)
}

// ── the rule itself ─────────────────────────────────────────────────────────

#[test]
fn a_directory_matches_the_project_its_basename_names() {
    let projects = vec![project_named("Alpha"), project_named("project-b")];

    assert_eq!(
        match_by_directory(
            &DirectoryIdentity::path_only("/home/u/src/project-b"),
            &projects,
        )
        .map(|p| p.name.as_str()),
        Some("project-b")
    );
    // The project name is slugified, so display casing and spacing still match.
    assert_eq!(
        match_by_directory(
            &DirectoryIdentity::path_only("/home/u/src/alpha"),
            &projects
        )
        .map(|p| p.name.as_str()),
        Some("Alpha")
    );
}

#[test]
fn a_trailing_separator_is_still_the_same_directory() {
    let projects = vec![project_named("Alpha")];
    assert_eq!(
        match_by_directory(
            &DirectoryIdentity::path_only("/home/u/src/alpha/"),
            &projects
        )
        .map(|p| p.name.as_str()),
        Some("Alpha"),
        "a path typed with a trailing slash names the same directory"
    );
}

#[test]
fn an_unrelated_directory_matches_nothing() {
    let projects = vec![project_named("Alpha")];
    assert!(
        match_by_directory(
            &DirectoryIdentity::path_only("/home/u/src/something-else"),
            &projects
        )
        .is_none()
    );
    assert!(
        match_by_directory(&DirectoryIdentity::path_only(""), &projects).is_none(),
        "a session with no path must not match by accident"
    );
}

// ── linking a session ───────────────────────────────────────────────────────

#[tokio::test]
async fn a_session_is_linked_when_its_directory_names_a_project() {
    let (project_svc, session_svc) = services().await;
    let project = project_svc
        .create_project("Alpha".to_string(), None)
        .await
        .unwrap();
    let session = session_svc.create_session(None).await.unwrap();
    session_svc
        .update_session_working_directory(session.id, Some("/home/u/src/alpha".to_string()))
        .await
        .unwrap();
    let session = session_svc.get_session(session.id).await.unwrap().unwrap();

    let linked = project_svc
        .link_session_by_directory(&session)
        .await
        .unwrap();

    assert_eq!(linked.map(|p| p.id), Some(project.id));
    let after = session_svc.get_session(session.id).await.unwrap().unwrap();
    assert_eq!(after.project_id, Some(project.id));
}

#[tokio::test]
async fn a_session_whose_directory_matches_nothing_is_left_alone() {
    let (project_svc, session_svc) = services().await;
    project_svc
        .create_project("Alpha".to_string(), None)
        .await
        .unwrap();
    let session = session_svc.create_session(None).await.unwrap();
    session_svc
        .update_session_working_directory(session.id, Some("/home/u/src/unrelated".to_string()))
        .await
        .unwrap();
    let session = session_svc.get_session(session.id).await.unwrap().unwrap();

    assert!(
        project_svc
            .link_session_by_directory(&session)
            .await
            .unwrap()
            .is_none()
    );
    let after = session_svc.get_session(session.id).await.unwrap().unwrap();
    assert_eq!(after.project_id, None);
}

#[tokio::test]
async fn an_already_assigned_session_is_never_reassigned() {
    let (project_svc, session_svc) = services().await;
    let alpha = project_svc
        .create_project("Alpha".to_string(), None)
        .await
        .unwrap();
    let beta = project_svc
        .create_project("Beta".to_string(), None)
        .await
        .unwrap();

    let session = session_svc.create_session(None).await.unwrap();
    session_svc
        .update_session_working_directory(session.id, Some("/home/u/src/alpha".to_string()))
        .await
        .unwrap();
    // Deliberately filed somewhere its directory does not name.
    project_svc
        .assign_session(session.id, beta.id)
        .await
        .unwrap();
    let session = session_svc.get_session(session.id).await.unwrap().unwrap();

    assert!(
        project_svc
            .link_session_by_directory(&session)
            .await
            .unwrap()
            .is_none(),
        "an explicit assignment is a decision; the directory rule must not overrule it"
    );
    let after = session_svc.get_session(session.id).await.unwrap().unwrap();
    assert_eq!(after.project_id, Some(beta.id));
    assert_ne!(after.project_id, Some(alpha.id));
}

#[tokio::test]
async fn a_channel_session_with_no_directory_is_skipped() {
    let (project_svc, session_svc) = services().await;
    project_svc
        .create_project("Alpha".to_string(), None)
        .await
        .unwrap();
    // Channel sessions carry no path at all, so no directory rule can classify
    // them and none should try.
    let session = session_svc.create_session(None).await.unwrap();

    assert!(
        project_svc
            .link_session_by_directory(&session)
            .await
            .unwrap()
            .is_none()
    );
}

// ── backfill ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_backfill_links_existing_sessions_and_is_idempotent() {
    let (project_svc, session_svc) = services().await;
    let alpha = project_svc
        .create_project("Alpha".to_string(), None)
        .await
        .unwrap();

    // Two that should link, one that should not, mirroring an install where
    // the project was created after the sessions.
    for dir in ["/home/u/src/alpha", "/home/u/other/alpha"] {
        let s = session_svc.create_session(None).await.unwrap();
        session_svc
            .update_session_working_directory(s.id, Some(dir.to_string()))
            .await
            .unwrap();
    }
    let stray = session_svc.create_session(None).await.unwrap();
    session_svc
        .update_session_working_directory(stray.id, Some("/home/u/src/unrelated".to_string()))
        .await
        .unwrap();

    assert_eq!(project_svc.backfill_unassigned_sessions().await.unwrap(), 2);
    assert_eq!(
        project_svc.backfill_unassigned_sessions().await.unwrap(),
        0,
        "a second sweep must find nothing left to do"
    );

    let after = session_svc.get_session(stray.id).await.unwrap().unwrap();
    assert_eq!(after.project_id, None);
    assert_eq!(
        project_svc
            .get_sessions_for_project(alpha.id)
            .await
            .unwrap()
            .len(),
        2
    );
}

// ── the ordering the fix depends on ─────────────────────────────────────────

#[test]
fn the_link_runs_outside_the_auto_title_future() {
    // The title is an LLM call that can fail, time out, or hit a rate-limited
    // provider. The link is a string match and one UPDATE. If the link were
    // nested inside the spawned title future, a provider outage would leave
    // sessions unlinked, which is the failure this change exists to fix. Pin
    // the ordering, since nothing else would notice it being moved.
    let src = std::fs::read_to_string("src/brain/agent/service/tool_loop.rs")
        .expect("tool_loop.rs must be readable");
    let link = src
        .find("link_session_by_directory")
        .expect("the first-turn link must still be here");
    let spawn = src[..link]
        .rfind("tokio::spawn")
        .map(|at| src[at..link].contains("auto_title"))
        .unwrap_or(false);
    assert!(
        !spawn,
        "the project link must not sit inside the spawned auto-title future"
    );
}

// ── linking must never restamp recency (#1460) ──────────────────────────────
//
// The first boot after the backfill shipped linked every session whose
// working directory named a project — and `assign_session` stamped
// `updated_at = now` on each row, so 60 sessions, including months-dead
// cron tombstones and zero-message "recovered" rows, all carried the same
// boot-second stamp and floated above genuinely recent sessions in
// `/sessions`. Project membership is derived metadata, not conversation
// activity: these tests pin that neither assign, unassign, nor the
// startup sweep touch `updated_at`. Same lesson the scope-all model
// writes learned in #1367.

/// Backdate `updated_at` to deterministic, staggered values via direct SQL,
/// so a shared bulk restamp cannot pass by accident. The connection handle
/// is scoped per iteration — the in-memory pool is single-conn, and holding
/// it across service calls deadlocks (see Database::connect_in_memory).
async fn backdate_updated_at(svc: &SessionService, stamps: &[(uuid::Uuid, i64)]) {
    use rusqlite::params;
    for (id, stamp) in stamps {
        let id = id.to_string();
        let stamp = *stamp;
        let conn = svc.pool().get().await.expect("conn");
        conn.interact(move |c| {
            c.execute(
                "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
                params![stamp, id],
            )
        })
        .await
        .expect("interact")
        .expect("backdate updated_at");
    }
}

#[tokio::test]
async fn assign_and_unassign_never_stamp_updated_at() {
    let (project_svc, session_svc) = services().await;
    let project = project_svc
        .create_project("Alpha".to_string(), None)
        .await
        .unwrap();
    let session = session_svc.create_session(None).await.unwrap();
    let stamp = 1_700_000_000i64;
    backdate_updated_at(&session_svc, &[(session.id, stamp)]).await;

    project_svc
        .assign_session(session.id, project.id)
        .await
        .unwrap();
    let row = session_svc.get_session(session.id).await.unwrap().unwrap();
    assert_eq!(row.project_id, Some(project.id), "assignment must land");
    assert_eq!(
        row.updated_at.timestamp(),
        stamp,
        "assign_session is a metadata write: updated_at must survive byte-identical (#1460)"
    );

    project_svc.unassign_session(session.id).await.unwrap();
    let row = session_svc.get_session(session.id).await.unwrap().unwrap();
    assert_eq!(row.project_id, None, "unassignment must land");
    assert_eq!(
        row.updated_at.timestamp(),
        stamp,
        "unassign_session must not restamp updated_at either (#1460)"
    );
}

#[tokio::test]
async fn the_backfill_links_without_restamping_recency() {
    let (project_svc, session_svc) = services().await;
    let project = project_svc
        .create_project("Alpha".to_string(), None)
        .await
        .unwrap();
    let a = session_svc.create_session(None).await.unwrap();
    let b = session_svc.create_session(None).await.unwrap();
    for id in [a.id, b.id] {
        session_svc
            .update_session_working_directory(id, Some("/home/u/src/alpha".to_string()))
            .await
            .unwrap();
    }
    // Staggered so any per-row "now" stamp fails the assertion, not just a
    // single shared timestamp.
    let stamps = [(a.id, 1_700_000_001i64), (b.id, 1_700_000_002i64)];
    backdate_updated_at(&session_svc, &stamps).await;

    let linked = project_svc.backfill_unassigned_sessions().await.unwrap();
    assert_eq!(linked, 2, "both sessions match the project by directory");

    for (id, expected) in &stamps {
        let row = session_svc.get_session(*id).await.unwrap().unwrap();
        assert_eq!(row.project_id, Some(project.id), "session must be linked");
        assert_eq!(
            row.updated_at.timestamp(),
            *expected,
            "the sweep set project_id but must not restamp updated_at (#1460)"
        );
    }
}
