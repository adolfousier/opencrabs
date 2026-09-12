//! The second identity: linking sessions to projects by repository remote
//! instead of by directory basename (#1510).
//!
//! A basename conflates unrelated checkouts that share a name — a benchmark
//! copy of one project sitting under another project's name used to link to
//! the wrong project and restamp cost rollups on it. `projects.repo_remote`
//! gives a project an identity its name cannot fake: the normalized origin
//! remote adopted the first time a real checkout proves it.

use crate::db::Database;
use crate::db::models::Project;
use crate::services::project_match::{
    DirectoryIdentity, match_by_directory, normalize_remote, resolve_directory_identity,
};
use crate::services::{ProjectService, ServiceContext, SessionService};

fn project(name: &str, remote: Option<&str>) -> Project {
    let mut p = Project::new(name.to_string(), None);
    p.repo_remote = remote.map(|r| r.to_string());
    p
}

fn identity(match_directory: &str, remote: Option<&str>) -> DirectoryIdentity {
    DirectoryIdentity {
        match_directory: match_directory.to_string(),
        remote: remote.map(|r| r.to_string()),
    }
}

fn git(args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

async fn services() -> (ProjectService, SessionService) {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let context = ServiceContext::new(db.pool().clone());
    (
        ProjectService::new(context.clone()),
        SessionService::new(context),
    )
}

// ── normalization ───────────────────────────────────────────────────────────

#[test]
fn every_way_of_cloning_the_same_repo_agrees_on_its_identity() {
    let canonical = normalize_remote("https://github.com/acme/widgets.git").unwrap();
    assert_eq!(canonical, "github.com/acme/widgets");
    for alt in [
        "git@github.com:acme/widgets.git",
        "ssh://git@github.com/acme/widgets",
        "git+ssh://git@github.com/acme/widgets.git",
        "https://GitHub.com/acme/widgets/",
        "http://github.com/acme/widgets",
    ] {
        assert_eq!(
            normalize_remote(alt).as_deref(),
            Some(canonical.as_str()),
            "{alt} must normalize to the same identity"
        );
    }
}

#[test]
fn the_path_keeps_its_case_and_a_different_repo_stays_different() {
    assert_ne!(
        normalize_remote("github.com/acme/Widgets"),
        normalize_remote("github.com/acme/widgets"),
        "case-sensitive forges must not be conflated"
    );
    assert_ne!(
        normalize_remote("github.com/acme/widgets"),
        normalize_remote("gitlab.com/acme/widgets"),
        "different hosts are different identities"
    );
}

#[test]
fn a_remote_without_a_host_has_no_identity_and_falls_through() {
    assert!(normalize_remote("").is_none());
    assert!(
        normalize_remote("/srv/git/widgets.git").is_none(),
        "an absolute local path has no host to compare; basename decides"
    );
}

// ── the two-identity rule ───────────────────────────────────────────────────

#[test]
fn a_matching_remote_links_from_any_directory_name() {
    let remote = "github.com/acme/widgets";
    let projects = vec![
        project("other", None),
        project("widgets-canonical", Some(remote)),
    ];
    assert_eq!(
        match_by_directory(
            &identity("/home/u/weird-checkout-name", Some(remote)),
            &projects
        )
        .map(|p| p.name.as_str()),
        Some("widgets-canonical"),
        "the remote identifies the project; neither directory name matters"
    );
}

#[test]
fn an_unproven_checkout_cannot_conflate_the_canonical_project() {
    // The benchmark collision, in both directions.
    let projects = vec![project("widgets", Some("github.com/acme/widgets"))];
    assert!(
        match_by_directory(&identity("/home/u/benchs/widgets", None), &projects).is_none(),
        "a silent basename must not override a project whose identity is a \
         remote it cannot share"
    );
    assert!(
        match_by_directory(
            &identity(
                "/home/u/forks/widgets",
                Some("github.com/elsewhere/widgets")
            ),
            &projects
        )
        .is_none(),
        "a proven-but-different repository must not link either"
    );
    assert!(
        match_by_directory(
            &identity("/home/u/benchs/widgets", Some("github.com/acme/widgets")),
            &projects
        )
        .is_some(),
        "a benchmark that IS the canonical repository still links: the guard \
         refuses unproven matches, not benchmarking"
    );
}

#[test]
fn basename_still_works_while_a_project_has_no_remote() {
    let projects = vec![project("Alpha", None)];
    assert_eq!(
        match_by_directory(&identity("/home/u/src/alpha", None), &projects)
            .map(|p| p.name.as_str()),
        Some("Alpha"),
        "plain directories and freshly created projects keep the pre-#1510 behavior"
    );
    assert_eq!(
        match_by_directory(
            &identity("/home/u/src/alpha", Some("github.com/anyone/alpha")),
            &projects
        )
        .map(|p| p.name.as_str()),
        Some("Alpha"),
        "a session matching an unclaimed project by name links and is the \
         candidate that records the remote"
    );
}

#[test]
fn a_proven_remote_outranks_an_unclaimed_basename_neighbor() {
    // The session directory is named after an unclaimed project, but the
    // repository is the one some other project has adopted. The stronger
    // identity wins.
    let projects = vec![
        project("opencrabs", None),
        project("crabsland-fork", Some("github.com/acme/opencrabs")),
    ];
    assert_eq!(
        match_by_directory(
            &identity("/home/u/src/opencrabs", Some("github.com/acme/opencrabs")),
            &projects
        )
        .map(|p| p.name.as_str()),
        Some("crabsland-fork")
    );
}

#[test]
fn resolve_names_the_repository_toplevel_not_the_folder_inside_it() {
    let dir = tempfile::TempDir::new().unwrap();
    let clone = dir.path().join("checkout-with-another-name");
    std::fs::create_dir(&clone).unwrap();
    let clone_path = clone.to_str().unwrap();
    if !git(&["-C", clone_path, "init", "-q"]) {
        return; // no git on this box; nothing to resolve
    }
    git(&[
        "-C",
        clone_path,
        "remote",
        "add",
        "origin",
        "https://github.com/acme/adopted.git",
    ]);
    let sub = clone.join("src/deep");
    std::fs::create_dir_all(&sub).unwrap();

    let resolved = resolve_directory_identity(sub.to_str().unwrap());
    assert_eq!(resolved.remote.as_deref(), Some("github.com/acme/adopted"));
    assert!(
        resolved
            .match_directory
            .ends_with("checkout-with-another-name"),
        "identity must be the repository root, not the subdirectory: {}",
        resolved.match_directory
    );
}

#[test]
fn a_path_with_no_repository_at_all_falls_back_to_its_basename() {
    // A path that does not exist fails the git probe before any upward walk,
    // so the identity stays the plain basename: exactly the pre-#1510 rule
    // for non-repository directories, and it must remain that way.
    let path = "/nonexistent/oc-1510-basename-dir";
    let resolved = resolve_directory_identity(path);
    assert_eq!(resolved.match_directory, path);
    assert!(resolved.remote.is_none());
    let projects = vec![project("oc-1510-basename-dir", None)];
    assert_eq!(
        match_by_directory(&resolved, &projects).map(|p| p.name.as_str()),
        Some("oc-1510-basename-dir"),
        "no repository is no obstacle to the basename rule"
    );
}

// ── adoption through the service ────────────────────────────────────────────

#[tokio::test]
async fn the_first_session_to_link_adopts_its_remote_for_the_project() {
    let (project_svc, session_svc) = services().await;
    let created = project_svc
        .create_project("adopted".to_string(), None)
        .await
        .unwrap();
    assert!(created.repo_remote.is_none());

    let dir = tempfile::TempDir::new().unwrap();
    let checkout = dir.path().join("adopted");
    std::fs::create_dir(&checkout).unwrap();
    let wd = checkout.to_str().unwrap().to_string();
    if !git(&["-C", &wd, "init", "-q"]) {
        return; // no git on this box
    }
    git(&[
        "-C",
        &wd,
        "remote",
        "add",
        "origin",
        "git@github.com:acme/adopted.git",
    ]);

    let session = session_svc.create_session(None).await.unwrap();
    session_svc
        .update_session_working_directory(session.id, Some(wd))
        .await
        .unwrap();
    let session = session_svc.get_session(session.id).await.unwrap().unwrap();

    let linked = project_svc
        .link_session_by_directory(&session)
        .await
        .unwrap()
        .expect("the basename match must link");
    assert_eq!(linked.id, created.id);

    let after = project_svc
        .list_projects()
        .await
        .unwrap()
        .into_iter()
        .find(|p| p.id == created.id)
        .unwrap();
    assert_eq!(
        after.repo_remote.as_deref(),
        Some("github.com/acme/adopted"),
        "linking a proven checkout records the repository for every session after it"
    );
}

#[tokio::test]
async fn a_recorded_remote_is_never_overwritten_by_the_next_session() {
    let (project_svc, session_svc) = services().await;
    let created = project_svc
        .create_project("adopted".to_string(), None)
        .await
        .unwrap();
    project_svc
        .set_project_repo_remote(created.id, "github.com/acme/adopted")
        .await
        .unwrap();

    // A session whose checkout proves a DIFFERENT repository cannot rewrite
    // the identity: the basename guard sends it nowhere instead.
    let dir = tempfile::TempDir::new().unwrap();
    let checkout = dir.path().join("adopted");
    std::fs::create_dir(&checkout).unwrap();
    let wd = checkout.to_str().unwrap().to_string();
    if !git(&["-C", &wd, "init", "-q"]) {
        return;
    }
    git(&[
        "-C",
        &wd,
        "remote",
        "add",
        "origin",
        "https://github.com/anyone/else.git",
    ]);
    let session = session_svc.create_session(None).await.unwrap();
    session_svc
        .update_session_working_directory(session.id, Some(wd))
        .await
        .unwrap();
    let session = session_svc.get_session(session.id).await.unwrap().unwrap();
    assert!(
        project_svc
            .link_session_by_directory(&session)
            .await
            .unwrap()
            .is_none(),
        "an unrelated same-named checkout must not link to the claimed project"
    );

    let after = project_svc
        .list_projects()
        .await
        .unwrap()
        .into_iter()
        .find(|p| p.id == created.id)
        .unwrap();
    assert_eq!(
        after.repo_remote.as_deref(),
        Some("github.com/acme/adopted")
    );
}
