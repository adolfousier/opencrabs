//! Receipt binding for the Ralph loop verification gate (#1011).
//!
//! A completion that claims "Committed <sha>" carries a factual assertion
//! the type-keyed verification commands cannot see: on 2026-08-11 a
//! type-'Edit' task claimed "Committed 7c1856c9", the sha never existed,
//! and the gate let it through as an Uncertain belief. The receipt check
//! verifies the claimed object exists in the session's repo: the commit
//! happened, or the claim is refused.
//!
//! Extraction is pure; verification is exercised against live fixture
//! repos (pattern from rsi_git_history_test.rs). Fixtures are synthetic
//! and carry no user identifiers.

use crate::brain::tools::plan_tool::{
    extract_sha_claims, receipt_binding_dir, validate_plan_working_directory, verify_sha_receipts,
};
use std::path::{Path, PathBuf};
use std::process::Command;

// ── Extraction: what counts as a commit claim ───────────────────────

#[test]
fn extracts_sha_after_committed() {
    // The smoking-gun shape from #1011.
    let claims = extract_sha_claims("Committed 7c1856c9: extracted classify_codex_failure");
    assert_eq!(claims, vec!["7c1856c9"]);
}

#[test]
fn extracts_across_commit_push_sha_keywords() {
    assert_eq!(
        extract_sha_claims("commit abc1234567 done"),
        vec!["abc1234567"]
    );
    assert_eq!(
        extract_sha_claims("pushed deadbeef01 to origin"),
        vec!["deadbeef01"]
    );
    assert_eq!(
        extract_sha_claims("sha cafebabe1234 verified"),
        vec!["cafebabe1234"]
    );
}

#[test]
fn keyword_matching_is_case_insensitive() {
    assert_eq!(
        extract_sha_claims("COMMITTED ABCDEF1234"),
        vec!["ABCDEF1234"]
    );
}

#[test]
fn ignores_hex_without_commit_context() {
    // Hex tokens are everywhere in build output; without a commit-ish
    // keyword nearby they are not claims.
    assert!(extract_sha_claims("error at deadbeef12345 in module").is_empty());
    // A hex slice of a longer identifier (tool call id) is not a claim.
    assert!(extract_sha_claims("call_eb7512b1c56a42f3a8d59009 accumulated").is_empty());
}

#[test]
fn ignores_file_hash_digests() {
    // sha256/sha512 context is a file hash, not a commit. The 64-char
    // digest also exceeds the 40-char cap.
    let text = "sha256 b23d70de9379c05b4b1ee2b7e1129dc2f41f45e0bf1d2c88c7a9b21c7d6e6a1f";
    assert!(extract_sha_claims(text).is_empty());
    // Truncated digest in sha256 context stays excluded by the keyword
    // guard even though it fits the sha length window.
    assert!(extract_sha_claims("sha256 b23d70de matches the artifact").is_empty());
}

#[test]
fn ignores_hex_shorter_than_git_abbrev() {
    assert!(extract_sha_claims("commit abc123").is_empty());
}

#[test]
fn dedupes_repeated_claims_case_insensitively() {
    let claims = extract_sha_claims("Committed 7c1856c9 and pushed 7C1856C9 to origin.");
    assert_eq!(claims, vec!["7c1856c9"]);
}

#[test]
fn empty_output_has_no_claims() {
    assert!(extract_sha_claims("").is_empty());
}

#[test]
fn multibyte_context_does_not_panic_the_window() {
    // The keyword window backs up 32 bytes; with multi-byte characters
    // before the sha the naive offset lands mid-character. Realignment
    // must keep this total, and the claim still resolves.
    let text = "ação ção ééé committed 7c1856c9 done";
    assert_eq!(extract_sha_claims(text), vec!["7c1856c9"]);
    // And the degenerate case: sha right after multi-byte noise, no
    // keyword at all — no panic, no claim.
    assert!(extract_sha_claims("éééééééééééééé 7c1856c9").is_empty());
}

// ── Verification: live git fixtures ─────────────────────────────────

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git available");
    assert!(
        out.status.success(),
        "git {:?} failed in {}: {}",
        args,
        repo.display(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
}

/// A fresh repo with one commit of `content`; returns (dir, sha). The
/// content parameter keeps two fixture repos from colliding on an
/// identical commit sha (same tree, same second).
fn fixture_repo_with_content(content: &[u8]) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("tmpdir");
    let repo = dir.path();
    git(repo, &["init", "-q"]);
    git(repo, &["config", "user.email", "receipt@test.local"]);
    git(repo, &["config", "user.name", "Receipt Test"]);
    std::fs::write(repo.join("fixture.txt"), content).expect("write fixture");
    git(repo, &["add", "fixture.txt"]);
    git(repo, &["commit", "-q", "-m", "fixture commit"]);
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse");
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (dir, sha)
}

/// A fresh repo with one commit; returns (dir, sha).
fn fixture_repo_with_commit() -> (tempfile::TempDir, String) {
    fixture_repo_with_content(b"receipt")
}

#[test]
fn real_sha_claim_passes() {
    let (dir, sha) = fixture_repo_with_commit();
    let output = format!("Committed {sha}: fixture work");
    assert!(verify_sha_receipts(&output, dir.path()).is_ok());
}

#[test]
fn real_short_prefix_claim_passes() {
    let (dir, sha) = fixture_repo_with_commit();
    let abbrev = &sha[..8];
    let output = format!("Committed {abbrev}: fixture work");
    assert!(verify_sha_receipts(&output, dir.path()).is_ok());
}

#[test]
fn phantom_sha_claim_is_rejected_with_evidence() {
    let (dir, _sha) = fixture_repo_with_commit();
    let phantom = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    let output = format!("Committed {phantom}: did the work");
    let err = verify_sha_receipts(&output, dir.path()).expect_err("phantom sha must reject");
    assert!(err.contains(phantom), "{err}");
    assert!(err.contains("#1011"), "{err}");
}

#[test]
fn unknown_short_prefix_is_rejected() {
    let (dir, _sha) = fixture_repo_with_commit();
    let err = verify_sha_receipts("Committed deadbee: done", dir.path())
        .expect_err("unknown prefix must reject");
    assert!(err.contains("deadbee"), "{err}");
}

#[test]
fn non_git_dir_skips_the_check() {
    // No receipt to demand outside a repo; completions in non-git
    // projects must keep working exactly as before.
    let dir = tempfile::tempdir().expect("tmpdir");
    let output = "Committed deadbeefdeadbeefdeadbeefdeadbeefdeadbeef: done";
    assert!(verify_sha_receipts(output, dir.path()).is_ok());
}

#[test]
fn output_without_claims_passes_in_a_repo() {
    let (dir, _sha) = fixture_repo_with_commit();
    assert!(verify_sha_receipts("Did the work, tests green", dir.path()).is_ok());
}

// ── #1452: binding resolves the repo from the plan working_directory ──

#[test]
fn binding_dir_prefers_the_plan_binding() {
    let resolved = receipt_binding_dir(Some("/plan/repo"), Path::new("/session/cwd"));
    assert_eq!(resolved, PathBuf::from("/plan/repo"));
}

#[test]
fn binding_dir_falls_back_to_session_cwd_without_a_binding() {
    let resolved = receipt_binding_dir(None, Path::new("/session/cwd"));
    assert_eq!(resolved, PathBuf::from("/session/cwd"));
}

#[test]
fn binding_dir_ignores_a_blank_binding() {
    let resolved = receipt_binding_dir(Some("   "), Path::new("/session/cwd"));
    assert_eq!(resolved, PathBuf::from("/session/cwd"));
}

#[test]
fn cross_repo_sha_verifies_against_the_plan_binding() {
    // The #1452 incident shape: session cwd is repo A, the plan's work
    // lives in repo B. Against A the B-sha is (correctly) missing — that
    // was the bug's face; the binding must resolve to B, where the sha
    // exists.
    let (repo_a, _sha_a) = fixture_repo_with_content(b"session repo work");
    let (repo_b, sha_b) = fixture_repo_with_content(b"plan repo work");
    let output = format!("Committed {sha_b}: did the work in the plan's repo");

    // Bug shape documented: checked against the session repo it fails.
    assert!(
        verify_sha_receipts(&output, repo_a.path()).is_err(),
        "sha_b must not exist in repo_a — if this fails the fixtures collided"
    );

    // The fix: the binding resolves to the plan's repo and accepts.
    let receipt_dir = receipt_binding_dir(
        Some(repo_b.path().to_str().expect("utf8 tempdir")),
        repo_a.path(),
    );
    assert!(verify_sha_receipts(&output, &receipt_dir).is_ok());
}

#[test]
fn validate_working_directory_rejects_relative_paths() {
    assert!(validate_plan_working_directory("srv/rs/bolina-rs").is_err());
    assert!(validate_plan_working_directory("./bolina-rs").is_err());
}

#[test]
fn validate_working_directory_rejects_missing_dirs() {
    assert!(validate_plan_working_directory("/definitely/not/a/real/dir_1452").is_err());
}

#[test]
fn validate_working_directory_accepts_an_existing_dir() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let path = dir.path().to_str().expect("utf8 tempdir");
    assert!(validate_plan_working_directory(path).is_ok());
}

#[test]
fn wiring_complete_passes_the_resolved_binding_dir() {
    // Source-level sentinel: the receipt-binding call site must consult
    // the plan's working_directory via receipt_binding_dir, not pass the
    // session cwd directly (the #1452 regression). Reverting the call
    // site to `verify_sha_receipts(&output, &context.working_dir())`
    // fails this test.
    let source = include_str!("../brain/tools/plan_tool.rs");
    assert!(
        source.contains("receipt_binding_dir("),
        "complete must resolve the verification repo via receipt_binding_dir (#1452)"
    );
    assert!(
        source.contains("current_plan.working_directory.as_deref()"),
        "complete must consult the plan's working_directory for the receipt binding (#1452)"
    );
}
