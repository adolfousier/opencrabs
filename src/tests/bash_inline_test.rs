//! Bash tool helpers.
//!
//! Moved out of `src/brain/tools/bash.rs`: tests live under `src/tests/`,
//! never inline beside the logic they exercise (#1076).

use crate::brain::tools::bash::*;

#[test]
fn grep_no_match_is_success() {
    // grep exit 1 + empty stderr = no matches found (successful empty search)
    assert!(is_search_no_match("grep foo file.txt", 1, ""));
    assert!(is_search_no_match("grep -r 'pattern' /path", 1, "   \n  "));
}

#[test]
fn grep_real_error_is_failure() {
    // grep exit 2 + stderr = real error (bad regex, missing file, etc.)
    assert!(!is_search_no_match(
        "grep foo file.txt",
        2,
        "grep: file.txt: No such file or directory"
    ));
    assert!(!is_search_no_match(
        "grep [invalid",
        1,
        "grep: invalid regex"
    ));
}

#[test]
fn non_grep_exit_1_is_failure() {
    // Non-grep commands exit 1 = real failure
    assert!(!is_search_no_match("ls /nonexistent", 1, ""));
    assert!(!is_search_no_match("cat missing.txt", 1, ""));
}

#[test]
fn piped_grep_no_match_is_success() {
    // Piped grep with no matches
    assert!(is_search_no_match("cat file.txt | grep foo", 1, ""));
    assert!(is_search_no_match("echo bar | grep baz", 1, ""));
}

#[test]
fn git_grep_no_match_is_success() {
    // git grep with no matches
    assert!(is_search_no_match("git grep 'nonexistent'", 1, ""));
    assert!(is_search_no_match("git grep -i pattern", 1, ""));
}

#[test]
fn rg_no_match_is_success() {
    // ripgrep with no matches
    assert!(is_search_no_match("rg 'pattern'", 1, ""));
    assert!(is_search_no_match("rg --type rust foo", 1, ""));
}

#[test]
fn egrep_fgrep_no_match_is_success() {
    // Extended/fixed grep variants
    assert!(is_search_no_match("egrep 'pattern'", 1, ""));
    assert!(is_search_no_match("fgrep 'literal'", 1, ""));
}

#[test]
fn findstr_no_match_is_success() {
    // Windows findstr: exit 1 + empty stderr = no lines matched, the same
    // contract as grep/rg. Counting it as failure turned every no-match
    // findstr into a phantom bash failure on Windows.
    assert!(is_search_no_match("findstr /i pattern file.txt", 1, ""));
    assert!(is_search_no_match("type log.txt | findstr ERROR", 1, "  \n"));
}

#[test]
fn findstr_real_error_is_failure() {
    // findstr exit 2 or with stderr = genuine error (bad syntax, etc.)
    assert!(!is_search_no_match(
        "findstr /i pattern file.txt",
        2,
        "FINDSTR: Cannot open file.txt"
    ));
    assert!(!is_search_no_match("findstr /f pattern", 1, "FINDSTR: Bad command line"));
}

#[test]
fn grep_as_argument_not_command() {
    // grep as an argument, not the command
    assert!(!is_search_no_match("echo grep", 1, ""));
    assert!(!is_search_no_match("cat file | wc -l", 1, ""));
}
