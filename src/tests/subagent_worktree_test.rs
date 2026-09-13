//! A sub-agent works in its own checkout, or in the parent's, never in a
//! half-isolated state.
//!
//! Children inherited the parent's directory, so a fan-out collided by
//! construction: several spawned at once wrote the same bytes with no
//! arbitration (#1151).

use std::process::Command;

use tempfile::TempDir;

use crate::brain::tools::subagent::worktree::create;

/// A real repository with one commit, since every operation here shells out to
/// git and a fake directory would prove nothing.
fn repo() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .expect("git runs");
    };
    run(&["init", "-q"]);
    run(&["config", "user.email", "test@example.invalid"]);
    run(&["config", "user.name", "Test"]);
    std::fs::write(dir.path().join("shared.txt"), "original\n").unwrap();
    run(&["add", "-A"]);
    run(&["commit", "-qm", "initial"]);
    dir
}

/// Run `f` with the profile home pointed at a throwaway directory.
///
/// Every test here needs it, because `create()` does not derive the worktree
/// path from the parent repo it is handed — it takes it from the profile home:
///
/// ```text
/// worktree_root() -> resolve_profile_home().join("worktrees")
/// ```
///
/// So passing an isolated `TempDir` as the parent *reads* as isolation and is
/// not: the checkout still landed in the developer's real
/// `~/.opencrabs/worktrees/`. Two of these tests keep their tree deliberately,
/// which means the leak was the assertion succeeding, and the leftovers then
/// failed the NEXT run by sending `create()` down its stale-path branch (#1535).
///
/// The override is task-local, so unlike pointing `$HOME` at a tempdir it stays
/// correct under a parallel run (#912).
fn in_isolated_home<T>(f: impl FnOnce() -> T) -> T {
    let home = TempDir::new().expect("tempdir");
    crate::config::profile::with_home_override(home.path().to_path_buf(), f)
}

#[test]
fn a_child_gets_a_checkout_of_its_own() {
    in_isolated_home(|| {
        let parent = repo();
        let wt = create(parent.path(), "aaaa1111").expect("worktree in a real repo");

        assert!(wt.path.is_dir(), "the child has somewhere to work");
        assert_ne!(wt.path, parent.path(), "and it is not the parent's tree");
        assert!(
            wt.path.join("shared.txt").is_file(),
            "with the repo's files"
        );
        assert_eq!(wt.branch, "subagent/aaaa1111");

        wt.cleanup();
    });
}

#[test]
fn two_children_editing_one_file_do_not_clobber_each_other() {
    in_isolated_home(|| {
        // The reported failure, reproduced: this is what sharing the parent's tree
        // could not do.
        let parent = repo();
        let a = create(parent.path(), "aaaa2222").expect("first child");
        let b = create(parent.path(), "bbbb2222").expect("second child");

        std::fs::write(a.path.join("shared.txt"), "written by A\n").unwrap();
        std::fs::write(b.path.join("shared.txt"), "written by B\n").unwrap();

        assert_eq!(
            std::fs::read_to_string(a.path.join("shared.txt")).unwrap(),
            "written by A\n"
        );
        assert_eq!(
            std::fs::read_to_string(b.path.join("shared.txt")).unwrap(),
            "written by B\n"
        );
        assert_eq!(
            std::fs::read_to_string(parent.path().join("shared.txt")).unwrap(),
            "original\n",
            "and the parent's tree is untouched by either"
        );

        a.cleanup();
        b.cleanup();
    });
}

#[test]
fn an_untouched_tree_is_returned() {
    in_isolated_home(|| {
        let parent = repo();
        let wt = create(parent.path(), "cccc3333").expect("worktree");
        let path = wt.path.clone();

        assert!(wt.cleanup(), "nothing was done in it, so it goes away");
        assert!(!path.is_dir(), "no orphan left behind");
    });
}

#[test]
fn a_tree_holding_uncommitted_work_is_kept() {
    in_isolated_home(|| {
        // Silently discarding a child's work would be worse than the collisions
        // this exists to prevent.
        let parent = repo();
        let wt = create(parent.path(), "dddd4444").expect("worktree");
        std::fs::write(wt.path.join("shared.txt"), "unsaved work\n").unwrap();

        assert!(wt.has_work());
        assert!(!wt.cleanup(), "kept rather than removed");
        assert!(wt.path.is_dir(), "and the work is still on disk");
    });
}

#[test]
fn a_tree_holding_a_commit_is_kept() {
    in_isolated_home(|| {
        let parent = repo();
        let wt = create(parent.path(), "eeee5555").expect("worktree");
        std::fs::write(wt.path.join("new.txt"), "committed work\n").unwrap();
        for args in [
            vec!["config", "user.email", "test@example.invalid"],
            vec!["config", "user.name", "Test"],
            vec!["add", "-A"],
            vec!["commit", "-qm", "child work"],
        ] {
            Command::new("git")
                .args(&args)
                .current_dir(&wt.path)
                .output()
                .expect("git runs");
        }

        assert!(wt.has_work(), "a commit counts as work");
        assert!(!wt.cleanup());
    });
}

#[test]
fn outside_a_repository_the_child_shares_the_parent_directory() {
    in_isolated_home(|| {
        // Not a failure: it is the behaviour that exists today, and a child that
        // cannot get its own tree must still run.
        let plain = TempDir::new().unwrap();
        assert!(create(plain.path(), "ffff6666").is_none());
    });
}

#[test]
fn a_kept_tree_is_named_in_what_the_parent_reads() {
    in_isolated_home(|| {
        // Isolation without this turns clobbering into stranded work: the child's
        // changes are real, they are not on the parent's branch, and nothing in
        // the result would say so.
        let parent = repo();
        let wt = create(parent.path(), "9999aaaa").expect("worktree");
        std::fs::write(wt.path.join("shared.txt"), "child work\n").unwrap();

        let removed = wt.cleanup();
        assert!(!removed, "a tree holding work is kept");

        let notice = wt.parent_notice(removed).expect("the parent is told");
        assert!(
            notice.contains("subagent/9999aaaa"),
            "names the branch: {notice}"
        );
        assert!(notice.contains("git merge"), "and how to land it: {notice}");
    });
}

#[test]
fn a_returned_tree_is_not_mentioned_at_all() {
    in_isolated_home(|| {
        // Most children change nothing. Reporting a branch for every one of them
        // would bury the cases that matter.
        let parent = repo();
        let wt = create(parent.path(), "8888bbbb").expect("worktree");

        let removed = wt.cleanup();
        assert!(removed);
        assert!(wt.parent_notice(removed).is_none());
    });
}

/// The isolation `in_isolated_home` provides, asserted rather than assumed.
///
/// Nothing in the eight tests above would notice losing it: they hand `create()`
/// an isolated parent repo and check the returned path, and every one of those
/// assertions passes just as well when the checkout is sitting in the
/// developer's real `~/.opencrabs/worktrees/`. That is how five fixture
/// directories ended up there (#1535) — the leak was silent because the tests
/// only ever ask about the tree, never about where it is.
#[test]
fn a_worktree_lands_under_the_overridden_home_and_not_the_real_one() {
    let home = TempDir::new().expect("tempdir");
    let parent = repo();
    let live = crate::config::profile::base_opencrabs_dir();

    let path = crate::config::profile::with_home_override(home.path().to_path_buf(), || {
        let wt = create(parent.path(), "7777cccc").expect("worktree in a real repo");
        let path = wt.path.clone();
        wt.cleanup();
        path
    });

    assert!(
        path.starts_with(home.path()),
        "the override must redirect the worktree root, got {}",
        path.display()
    );
    assert!(
        !path.starts_with(&live),
        "a test must never write into the real profile home: {}",
        path.display()
    );
}
