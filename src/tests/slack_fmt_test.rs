//! Tests for Markdown → Slack mrkdwn conversion.

#[cfg(test)]
mod tests {
    use crate::utils::slack_fmt::markdown_to_mrkdwn;

    #[test]
    fn test_bold_double_asterisk_to_single() {
        assert_eq!(markdown_to_mrkdwn("**bold text**"), "*bold text*");
    }

    #[test]
    fn test_bold_multiple_in_line() {
        assert_eq!(markdown_to_mrkdwn("**one** and **two**"), "*one* and *two*");
    }

    #[test]
    fn test_italic_unchanged() {
        assert_eq!(markdown_to_mrkdwn("_italic_"), "_italic_");
    }

    #[test]
    fn test_strikethrough_double_tilde_to_single() {
        assert_eq!(markdown_to_mrkdwn("~~deleted~~"), "~deleted~");
    }

    #[test]
    fn test_inline_code_unchanged() {
        assert_eq!(markdown_to_mrkdwn("`code`"), "`code`");
    }

    #[test]
    fn test_code_block_unchanged() {
        let input = "```rust\nfn main() {}\n```";
        assert_eq!(markdown_to_mrkdwn(input), input);
    }

    #[test]
    fn test_bold_inside_code_block_not_converted() {
        let input = "```\n**not bold**\n```";
        assert_eq!(markdown_to_mrkdwn(input), input);
    }

    #[test]
    fn test_bold_inside_inline_code_not_converted() {
        assert_eq!(markdown_to_mrkdwn("`**not bold**`"), "`**not bold**`");
    }

    #[test]
    fn test_heading_h1() {
        assert_eq!(markdown_to_mrkdwn("# Title"), "*Title*\n");
    }

    #[test]
    fn test_heading_h2() {
        assert_eq!(markdown_to_mrkdwn("## Section"), "*Section*\n");
    }

    #[test]
    fn test_heading_h3() {
        assert_eq!(markdown_to_mrkdwn("### Subsection"), "*Subsection*\n");
    }

    #[test]
    fn test_link_conversion() {
        assert_eq!(
            markdown_to_mrkdwn("[Google](https://google.com)"),
            "<https://google.com|Google>"
        );
    }

    #[test]
    fn test_link_with_bold() {
        assert_eq!(
            markdown_to_mrkdwn("**Check** [this](https://example.com)"),
            "*Check* <https://example.com|this>"
        );
    }

    #[test]
    fn test_plain_text_unchanged() {
        assert_eq!(markdown_to_mrkdwn("just plain text"), "just plain text");
    }

    #[test]
    fn test_empty_string() {
        assert_eq!(markdown_to_mrkdwn(""), "");
    }

    #[test]
    fn test_mixed_formatting() {
        let input = "**Issue 1: Confirmation popup too tall to scroll (50+ docs)**\n\n**Root cause:** `SubmitConfirmationDialog` has no max-height.";
        let expected = "*Issue 1: Confirmation popup too tall to scroll (50+ docs)*\n\n*Root cause:* `SubmitConfirmationDialog` has no max-height.";
        assert_eq!(markdown_to_mrkdwn(input), expected);
    }

    #[test]
    fn test_heading_then_content() {
        let input = "## Summary\n\nHere is the plan.";
        let expected = "*Summary*\n\nHere is the plan.";
        assert_eq!(markdown_to_mrkdwn(input), expected);
    }

    #[test]
    fn test_numbered_list_with_bold() {
        let input = "1. **First** item\n2. **Second** item";
        let expected = "1. *First* item\n2. *Second* item";
        assert_eq!(markdown_to_mrkdwn(input), expected);
    }

    #[test]
    fn test_single_asterisk_italic_unchanged() {
        // Single * is italic in some markdown — should pass through as-is (also bold in mrkdwn)
        assert_eq!(markdown_to_mrkdwn("*text*"), "*text*");
    }

    #[test]
    fn test_bracket_not_link() {
        // Square brackets without () should be left as-is
        assert_eq!(markdown_to_mrkdwn("[not a link]"), "[not a link]");
    }

    #[test]
    fn test_real_world_plan_message() {
        let input = r#"Here's the plan. Two separate issues, one straightforward fix, one needs deeper investigation:

---

**Issue 1: Confirmation popup too tall to scroll (50+ docs)**

**Root cause:** `SubmitConfirmationDialog` has no max-height or internal scroll on the document list. With 50 files, the list alone can push the "Confirm & Submit" button below the viewport.

**Fix:** Add internal scroll to the document list + cap at `max-h-48` with a "and X more files" summary. Buttons stay sticky.

**Files:** `client/components/dashboard/modals/SubmitConfirmationDialog.tsx`"#;

        let output = markdown_to_mrkdwn(input);

        // Bold should be converted
        assert!(
            !output.contains("**"),
            "Output still contains double asterisks: {}",
            output
        );
        // Single asterisk bold should be present
        assert!(output.contains("*Issue 1:"));
        assert!(output.contains("*Root cause:*"));
        // Code should be preserved
        assert!(output.contains("`SubmitConfirmationDialog`"));
        assert!(output.contains("`max-h-48`"));
    }
}
