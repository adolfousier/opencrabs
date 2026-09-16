//! Tests for `brain_file_safety` — the chokepoint that protects
//! `~/.opencrabs/*.md` brain files from agent-driven destruction.
//!
//! Background: 2026-04-26 the RSI agent shrank TOOLS.md from 33 KB to a
//! stub by passing the entire file as `old_content` to a
//! `self_improve` action="update". MEMORY.md got the same treatment.
//! The user policy is append-only — removals only when the bytes
//! disappearing are duplicates that survive elsewhere in the result.

use crate::brain::tools::brain_file_safety::{
    ShrinkCheck, backup_before_write, check_no_shrink, is_protected_brain_file, is_protected_path,
};
use std::path::Path;

mod protected_predicate {
    use super::*;

    #[test]
    fn known_brain_files_are_protected() {
        for name in [
            "SOUL.md",
            "USER.md",
            "AGENTS.md",
            "TOOLS.md",
            "CODE.md",
            "SECURITY.md",
            "MEMORY.md",
            "BOOT.md",
        ] {
            assert!(is_protected_brain_file(name), "{name} should be protected");
        }
    }

    #[test]
    fn case_insensitive_match() {
        // The agent sometimes lowercases names — protect either way.
        assert!(is_protected_brain_file("memory.md"));
        assert!(is_protected_brain_file("tools.md"));
    }

    #[test]
    fn unrelated_files_are_not_protected() {
        assert!(!is_protected_brain_file("commands.toml"));
        assert!(!is_protected_brain_file("notes.md"));
        assert!(!is_protected_brain_file("memory/2026-04-26.md"));
    }

    #[test]
    fn path_predicate_uses_basename() {
        assert!(is_protected_path(Path::new("/Users/x/.opencrabs/TOOLS.md")));
        assert!(is_protected_path(Path::new("TOOLS.md")));
        assert!(!is_protected_path(Path::new(
            "/Users/x/.opencrabs/memory/log.md"
        )));
    }
}

mod shrink_check {
    use super::*;

    fn protected() -> &'static Path {
        Path::new("/tmp/fake/TOOLS.md")
    }

    fn unprotected() -> &'static Path {
        Path::new("/tmp/fake/notes.md")
    }

    #[test]
    fn unprotected_file_is_always_allowed() {
        // Non-brain files can shrink freely — that's not what this guard
        // is for.
        let result = check_no_shrink(unprotected(), "lots of content here", "tiny", false, false);
        assert_eq!(result, ShrinkCheck::Allowed);
    }

    #[test]
    fn append_to_protected_is_allowed() {
        // The append-only happy path — content grows.
        let existing = "line one\nline two\n";
        let updated = "line one\nline two\nline three\n";
        assert_eq!(
            check_no_shrink(protected(), existing, updated, false, false),
            ShrinkCheck::Allowed
        );
    }

    #[test]
    fn equal_size_rewrite_is_allowed() {
        // Same length update (e.g. typo fix) doesn't shrink, so it's
        // allowed even on a protected file.
        let existing = "abc def ghi";
        let updated = "abc XYZ ghi";
        assert_eq!(
            check_no_shrink(protected(), existing, updated, false, false),
            ShrinkCheck::Allowed
        );
    }

    #[test]
    fn shrink_without_cleanup_intent_is_rejected() {
        // The 2026-04-26 case: agent rewrites the whole file from many
        // KB down to a stub. Hard reject.
        let existing = "rule one\nrule two\nrule three\nrule four\nrule five\n";
        let updated = "rule one\n";
        match check_no_shrink(protected(), existing, updated, false, false) {
            ShrinkCheck::Rejected { message } => {
                assert!(message.contains("Refusing to shrink"));
                assert!(message.contains("TOOLS.md"));
                assert!(message.contains("append-only"));
            }
            other => panic!("expected Rejected, got {:?}", other),
        }
    }
}

mod cleanup_intent {
    use super::*;

    fn protected() -> &'static Path {
        Path::new("/tmp/fake/TOOLS.md")
    }

    #[test]
    fn cleanup_intent_allows_shrinking_protected_file() {
        // User explicitly wants to clean up a brain file — allow it.
        let existing = "rule one\nrule two\nrule three\nrule four\nrule five\n";
        let updated = "rule one\n";
        assert_eq!(
            check_no_shrink(protected(), existing, updated, true, false),
            ShrinkCheck::Allowed
        );
    }

    #[test]
    fn cleanup_intent_allows_line_deletions() {
        // cleanup_intent allows deleting arbitrary lines.
        let existing = "alpha\nbeta\ngamma\ndelta\n";
        let updated = "only one line\n";
        assert_eq!(
            check_no_shrink(protected(), existing, updated, true, false),
            ShrinkCheck::Allowed
        );
    }

    #[test]
    fn cleanup_intent_false_still_enforces_append_only() {
        // When cleanup_intent is false, the append-only rule still applies.
        let existing = "rule one\nrule two\nrule three\n";
        let updated = "rule one\n";
        match check_no_shrink(protected(), existing, updated, false, false) {
            ShrinkCheck::Rejected { message } => {
                assert!(message.contains("Refusing to shrink"));
            }
            other => panic!("expected Rejected, got {:?}", other),
        }
    }

    #[test]
    fn cleanup_intent_on_unprotected_file() {
        // Unprotected files can shrink regardless — cleanup_intent is irrelevant.
        let unprotected = Path::new("/tmp/fake/notes.md");
        let existing = "lots of content here";
        let updated = "tiny";
        assert_eq!(
            check_no_shrink(unprotected, existing, updated, true, false),
            ShrinkCheck::Allowed
        );
    }
}

mod backup {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn skips_backup_for_nonexistent_file() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("doesnotexist.md");
        let result = backup_before_write(&path).expect("ok");
        assert!(result.is_none());
    }

    #[test]
    fn backs_up_existing_file_to_timestamped_path() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("TOOLS.md");
        std::fs::write(&path, "important content").expect("seed");

        let backup = backup_before_write(&path)
            .expect("ok")
            .expect("backup path returned");

        assert!(backup.exists(), "backup file should exist");
        let backup_name = backup.file_name().unwrap().to_string_lossy().to_string();
        assert!(
            backup_name.starts_with("TOOLS.md."),
            "backup name should start with original: {backup_name}"
        );
        assert!(
            backup_name.ends_with(".bak"),
            "backup name should end with .bak: {backup_name}"
        );
        let backed_up = std::fs::read_to_string(&backup).expect("read backup");
        assert_eq!(backed_up, "important content");

        // Original is untouched
        let original = std::fs::read_to_string(&path).expect("read original");
        assert_eq!(original, "important content");
    }
}

mod dedup_append {
    use crate::brain::tools::brain_file_safety::is_duplicate_append;

    #[test]
    fn empty_content_is_duplicate() {
        assert!(is_duplicate_append("existing content", ""));
        assert!(is_duplicate_append("existing content", "   "));
    }

    #[test]
    fn exact_match_is_duplicate() {
        let existing = "## Server Info\nHost: localhost\nPort: 8080\n";
        assert!(is_duplicate_append(
            existing,
            "## Server Info\nHost: localhost\nPort: 8080\n"
        ));
    }

    #[test]
    fn substring_match_is_duplicate() {
        let existing = "alpha\nbeta\ngamma\ndelta\n";
        assert!(is_duplicate_append(existing, "beta\ngamma\n"));
    }

    #[test]
    fn new_content_is_not_duplicate() {
        let existing = "## Server Info\nHost: localhost\n";
        assert!(!is_duplicate_append(
            existing,
            "## Database\nHost: db.example.com\n"
        ));
    }

    #[test]
    fn same_section_header_is_duplicate() {
        let existing = "## Server Info\nOld content here\n";
        assert!(is_duplicate_append(
            existing,
            "## Server Info\nNew content here\n"
        ));
    }

    #[test]
    fn same_subsection_header_is_duplicate() {
        let existing = "## Servers\n### Web Server\nDetails here\n";
        assert!(is_duplicate_append(
            existing,
            "### Web Server\nUpdated details\n"
        ));
    }

    #[test]
    fn different_subsection_is_not_duplicate() {
        let existing = "## Servers\n### Web Server\nDetails here\n";
        assert!(!is_duplicate_append(
            existing,
            "### DB Server\nNew details\n"
        ));
    }

    #[test]
    fn high_line_overlap_is_duplicate() {
        let existing = "line one\nline two\nline three\nline four\nline five\n";
        let new_content = "line one\nline two\nline three\nline six\n";
        // 3 out of 4 non-empty lines overlap = 75% > 60%
        assert!(is_duplicate_append(existing, new_content));
    }

    #[test]
    fn low_line_overlap_is_not_duplicate() {
        let existing = "line one\nline two\nline three\n";
        let new_content = "line one\nnew alpha\nnew beta\nnew gamma\n";
        // 1 out of 4 = 25% < 60%
        assert!(!is_duplicate_append(existing, new_content));
    }

    #[test]
    fn empty_existing_file_is_not_duplicate() {
        assert!(!is_duplicate_append("", "## New Section\nFresh content\n"));
    }

    #[test]
    fn whitespace_trimmed_for_comparison() {
        let existing = "## Server\nHost: localhost\n";
        assert!(is_duplicate_append(
            existing,
            "  ## Server\nHost: localhost\n  "
        ));
    }
}

mod filter_duplicate_append {
    use crate::brain::tools::brain_file_safety::{AppendDedup, filter_duplicate_append};

    #[test]
    fn all_new_content_passes_through() {
        let existing = "## Servers\nWeb: localhost\n";
        let result = filter_duplicate_append(existing, "## Database\nHost: db.local\nPort: 5432\n");
        assert!(matches!(result, AppendDedup::AllNew));
    }

    #[test]
    fn fully_duplicate_content_is_blocked() {
        let existing = "## Servers\nWeb: localhost\n";
        let result = filter_duplicate_append(existing, "## Servers\nWeb: localhost\n");
        assert!(matches!(result, AppendDedup::AllDuplicate));
    }

    #[test]
    fn partially_new_filters_correctly() {
        let existing = "## Servers\nWeb: localhost\n\n## Ports\nHTTP: 80\n";
        // First paragraph already exists, second is new
        let new_content = "## Servers\nWeb: localhost\n\n## Database\nHost: db.local\n";
        match filter_duplicate_append(existing, new_content) {
            AppendDedup::Filtered {
                filtered_content,
                skipped_paragraphs,
            } => {
                assert_eq!(skipped_paragraphs, 1);
                assert!(filtered_content.contains("## Database"));
                assert!(!filtered_content.contains("## Servers"));
            }
            other => panic!("expected Filtered, got {:?}", other),
        }
    }

    #[test]
    fn three_paragraphs_two_existing() {
        let existing = "## Alpha\nalpha content\n\n## Bravo\nbravo content\n";
        // Alpha and Bravo exist, Charlie is new
        let new_content =
            "## Alpha\nalpha content\n\n## Bravo\nbravo content\n\n## Charlie\ncharlie content\n";
        match filter_duplicate_append(existing, new_content) {
            AppendDedup::Filtered {
                filtered_content,
                skipped_paragraphs,
            } => {
                assert_eq!(skipped_paragraphs, 2);
                assert!(filtered_content.contains("## Charlie"));
                assert!(!filtered_content.contains("## Alpha"));
                assert!(!filtered_content.contains("## Bravo"));
            }
            other => panic!("expected Filtered, got {:?}", other),
        }
    }

    #[test]
    fn empty_existing_file_allows_everything() {
        let result = filter_duplicate_append(
            "",
            "## New Section\nFresh content\n\n## Another\nMore content\n",
        );
        assert!(matches!(result, AppendDedup::AllNew));
    }

    #[test]
    fn empty_new_content_is_all_duplicate() {
        let result = filter_duplicate_append("existing", "");
        assert!(matches!(result, AppendDedup::AllDuplicate));
    }

    #[test]
    fn same_header_different_body_is_filtered() {
        let existing = "## Config\nold value\n";
        let new_content = "## Config\nnew value\n";
        // Same header → paragraph_exists returns true
        let result = filter_duplicate_append(existing, new_content);
        assert!(matches!(result, AppendDedup::AllDuplicate));
    }

    #[test]
    fn single_paragraph_all_new() {
        let existing = "## Alpha\nalpha\n";
        let result = filter_duplicate_append(existing, "## Beta\nbeta\n");
        assert!(matches!(result, AppendDedup::AllNew));
    }
}
