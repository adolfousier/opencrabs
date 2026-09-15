//! Tests for the processing-log flow renderers.
//!
//! Classic path: grouped tool calls render as native Telegram HTML using
//! `<blockquote expandable>` (Bot API 7.3+), never `<details>` — Telegram's
//! regular HTML parse mode does not support `<details>`, so that tag leaks
//! into the chat as literal text. Rich path (#420 path A): the rich API's
//! HTML INPUT mode is different — there `<details><summary>` is parsed
//! server-side into a native RichBlockDetails collapsible, so
//! `render_flow_details` emits exactly that wrapper.

use crate::channels::telegram::flow::{
    FlowEntry, FlowHeader, FlowOutcome, HeaderMarkup, extract_status_from_text, flow_header_text,
    humanize_duration, latest_activity_preview, pop_trailing_folded_texts,
    render_flow_details_with, render_flow_html_with,
};
use crate::channels::telegram::handler::{
    FlowLine, folded_duplicates_final, render_flow_details, render_flow_html, render_flow_rich,
};

fn tline(label: &str, context: &str) -> FlowLine {
    FlowLine::Tool {
        label: label.to_string(),
        context: context.to_string(),
        raw_context: String::new(),
    }
}

/// Build a bash flow line the way production does (#488): `context` is the
/// decorated, middle-truncated display hint and `raw_context` is the untruncated
/// command. Status extraction must read `raw_context`, so tests that assert
/// `#`-comment status go through this, not `tline` (which passed the raw command
/// as `context` and so tested input the pipeline never produces).
fn bash_line(raw_command: &str) -> FlowLine {
    let input = serde_json::json!({ "command": raw_command });
    FlowLine::Tool {
        label: "⚙️ bash".to_string(),
        context: crate::utils::tool_context_hint("bash", &input),
        raw_context: crate::utils::tool_status_source("bash", &input),
    }
}

#[test]
fn empty_group_renders_footer_only() {
    // ADR 0005 F1: with no entries there is no log block, so the classic message
    // collapses to the plain merged footer, which with elapsed_secs=0 (the
    // wrapper passes 0) is just the clock. The legacy live-status argument no
    // longer feeds the footer.
    assert_eq!(render_flow_html(&[], None), "⏱ 0:00");
    assert_eq!(render_flow_html(&[], Some("45s")), "⏱ 0:00");
}

#[test]
fn single_tool_renders_block() {
    // ADR 0005 F1: a single tool is no longer a plain one-liner — it renders as
    // a normal log block, the entry inside the blockquote and the merged footer
    // (status/activity + count + clock) on the plain line below.
    let out = render_flow_html(&[tline("✅ bash", "git status")], None);
    assert_eq!(
        out,
        "<blockquote expandable><b>✅ bash</b> <code>git status</code></blockquote>\n\
         ⚙️ bash git status • 1 tool calls • ⏱ 0:00"
    );
    assert!(out.contains("<blockquote expandable>"));
}

#[test]
fn single_tool_without_context_omits_trailing_space() {
    // Empty context → the entry body is just the bold label, no trailing
    // <code>/space, even now that a lone tool renders as a block.
    let out = render_flow_html(&[tline("⚙️ web_search", "")], None);
    assert!(out.contains("<blockquote expandable><b>⚙️ web_search</b></blockquote>"));
    assert!(!out.contains("<b>⚙️ web_search</b> "));
    assert!(!out.contains("<code>"));
}

#[test]
fn multiple_tools_render_expandable_blockquote() {
    let out = render_flow_html(
        &[
            tline("✅ bash", "cargo fmt"),
            tline("✅ read_file", "handler.rs"),
            tline("❌ grep", "pattern"),
        ],
        None,
    );
    // ADR 0005 F1: the blockquote holds only the entry bodies; the status +
    // activity + count live in the merged footer (plain line after the block).
    assert!(out.starts_with("<blockquote expandable><b>✅ bash</b> <code>cargo fmt</code>\n\n"));
    assert!(out.contains("</blockquote>\n"));
    let footer = out.rsplit('\n').next().unwrap();
    assert_eq!(footer, "⚙️ grep pattern • 3 tool calls • ⏱ 0:00");
    assert!(out.contains("<b>✅ bash</b> <code>cargo fmt</code>"));
    assert!(out.contains("<b>✅ read_file</b> <code>handler.rs</code>"));
    assert!(out.contains("<b>❌ grep</b> <code>pattern</code>"));
}

#[test]
fn blocks_are_separated_by_blank_lines() {
    let out = render_flow_html(
        &[
            tline("✅ bash", "cargo fmt"),
            FlowLine::Text("Reformatted three files.".to_string()),
            tline("✅ read_file", "handler.rs"),
        ],
        None,
    );
    // ADR 0005 F1: entries sit inside the blockquote separated by blank lines;
    // the narration-led status/count is in the merged footer, not a header row.
    assert!(out.starts_with("<blockquote expandable><b>✅ bash</b> <code>cargo fmt</code>\n\n"));
    let footer = out.rsplit('\n').next().unwrap();
    assert_eq!(
        footer,
        "⚙️ Reformatted three files. • 2 tool calls • ⏱ 0:00"
    );
    assert!(out.contains("<b>✅ bash</b> <code>cargo fmt</code>\n\nReformatted three files."));
    assert!(
        out.contains("Reformatted three files.\n\n<b>✅ read_file</b> <code>handler.rs</code>")
    );
}

#[test]
fn tool_context_renders_as_monospace() {
    // Paths / commands / queries read as code, not prose, in the expanded block.
    let out = render_flow_html(
        &[
            tline("✅ read", "src/channels/telegram/handler.rs"),
            tline("✅ bash", "cargo clippy --all-features"),
        ],
        None,
    );
    assert!(out.contains("<b>✅ read</b> <code>src/channels/telegram/handler.rs</code>"));
    assert!(out.contains("<b>✅ bash</b> <code>cargo clippy --all-features</code>"));
}

#[test]
fn intermediate_text_renders_inline_markdown() {
    // Narration folded into the block gets the same inline markdown as the final
    // completion below it: `code` spans, **bold**, *italic* render, not raw.
    let out = render_flow_html(
        &[
            tline("✅ bash", "grep foo"),
            FlowLine::Text("Calling `analyze_image` then **committing** the *fix*.".to_string()),
        ],
        None,
    );
    assert!(out.contains("<code>analyze_image</code>"));
    assert!(out.contains("<b>committing</b>"));
    assert!(out.contains("<i>fix</i>"));
    // No raw markdown markers survive — the body renders them and the
    // latest-activity preview strips them (#405).
    assert!(!out.contains("`analyze_image`"));
    assert!(!out.contains("**committing**"));
}

#[test]
fn never_emits_details_tags() {
    let out = render_flow_html(&[tline("✅ a", "x"), tline("✅ b", "y")], None);
    assert!(!out.contains("<details>"));
    assert!(!out.contains("<summary>"));
}

#[test]
fn escapes_html_in_labels_and_context() {
    let out = render_flow_html(
        &[
            tline("✅ bash", "grep '<details>' & \"stuff\""),
            tline("✅ edit_file", "a < b > c"),
        ],
        None,
    );
    assert!(out.contains("grep '&lt;details&gt;' &amp; \"stuff\""));
    assert!(out.contains("a &lt; b &gt; c"));
    // No raw angle brackets from content survive outside our own tags
    assert!(!out.contains("'<details>'"));
}

// ── latest_activity_preview: whole human-readable text, JSON/code skip (#481) ──

#[test]
fn preview_uses_whole_human_readable_text_untruncated() {
    // Amendment: the whole intermediary text is the status source — every
    // paragraph, newlines preserved, no 96-char cap.
    let long = "First paragraph of the plan.\nSecond line with more detail.\nThird line that pushes well past the old ninety-six character truncation limit so the whole thing survives.";
    let out = latest_activity_preview(&[
        tline("✅ bash", "cargo test"),
        FlowLine::Text(long.to_string()),
    ]);
    assert_eq!(out.as_deref(), Some(long));
}

#[test]
fn preview_skips_json_last_entry_back_to_narration() {
    // A raw-JSON last entry is not human-readable; the preview walks back to the
    // prior narration instead of showing `{"model": ...}`.
    let out = latest_activity_preview(&[
        FlowLine::Text("Checking the model roster.".to_string()),
        FlowLine::Text("{\"model\": \"deepseek-v4-flash\", \"ok\": true}".to_string()),
    ]);
    assert_eq!(out.as_deref(), Some("Checking the model roster."));
}

#[test]
fn preview_skips_code_block_last_entry() {
    let out = latest_activity_preview(&[
        FlowLine::Text("Here is the fix.".to_string()),
        FlowLine::Text("```rust\nfn main() {}\n```".to_string()),
    ]);
    assert_eq!(out.as_deref(), Some("Here is the fix."));
}

#[test]
fn preview_strips_inline_markdown_markers() {
    let out = latest_activity_preview(&[FlowLine::Text(
        "Calling `grep` then **committing** the *fix*.".to_string(),
    )]);
    assert_eq!(
        out.as_deref(),
        Some("Calling grep then committing the fix.")
    );
}

#[test]
fn preview_keeps_one_word_sentence_but_skips_bare_path() {
    // "Done." is narration (keeps letters); a bare path is raw output, skipped.
    assert_eq!(
        latest_activity_preview(&[FlowLine::Text("Done.".to_string())]).as_deref(),
        Some("Done.")
    );
    let out = latest_activity_preview(&[
        tline("✅ read_file", "handler.rs"),
        FlowLine::Text("src/channels/telegram/flow.rs".to_string()),
    ]);
    assert_eq!(out.as_deref(), Some("✅ read_file handler.rs"));
}

#[test]
fn preview_falls_back_to_tool_when_no_human_readable_text() {
    let out = latest_activity_preview(&[
        tline("✅ read_file", "handler.rs"),
        FlowLine::Text("[1,2,3]".to_string()),
    ]);
    assert_eq!(out.as_deref(), Some("✅ read_file handler.rs"));
}

#[test]
fn preview_is_none_when_empty() {
    assert_eq!(latest_activity_preview(&[]), None);
}

// ── extract_status_from_text: bash line-start # comments (#482) ──

#[test]
fn bash_comment_single_strips_decoration() {
    assert_eq!(
        extract_status_from_text("# --- Setup environment ---\nexport FOO=bar").as_deref(),
        Some("Setup environment")
    );
}

#[test]
fn bash_comment_multiple_join_with_newlines() {
    let cmd = "# Step one\napt-get update\n# Step two\napt-get install foo";
    assert_eq!(
        extract_status_from_text(cmd).as_deref(),
        Some("Step one\nStep two")
    );
}

#[test]
fn bash_comment_ignores_inline_hash_and_shebang() {
    // A shebang and an inline (not line-start) `#` are not status comments.
    let cmd = "#!/bin/bash\ncurl https://x.com/a#frag\necho ok";
    assert_eq!(extract_status_from_text(cmd), None);
}

#[test]
fn bash_comment_none_when_no_comments() {
    assert_eq!(extract_status_from_text("cargo build --release"), None);
}

#[test]
fn preview_uses_bash_comments_when_no_narration() {
    // No human-readable text → priority 2 pulls the bash command's comments.
    // bash_line (not tline) so the pipeline is realistic: context is decorated,
    // raw_context holds the raw command that extraction must read (#488).
    let out = latest_activity_preview(&[bash_line("# --- Installing deps ---\nnpm install")]);
    assert_eq!(out.as_deref(), Some("Installing deps"));
}

#[test]
fn preview_bash_comment_on_first_line_survives_decoration() {
    // The #488 regression: the display hint wraps the command as `` (`…`) ``, so
    // a comment on the command's FIRST line no longer starts with `#` after
    // decoration and was dropped. Reading raw_context restores it.
    let out = latest_activity_preview(&[bash_line("# --- Setup environment ---\nexport FOO=bar")]);
    assert_eq!(out.as_deref(), Some("Setup environment"));
}

#[test]
fn preview_bash_comment_survives_long_command_no_truncation() {
    // The display hint middle-truncates to 80 bytes, which elided comments in a
    // long command (#488). raw_context is untruncated, so a trailing comment
    // past the 80-byte budget still surfaces.
    let long = format!(
        "# Build step\n{}\n# Deploy step\nmake deploy",
        "echo padding-".repeat(12)
    );
    assert!(long.len() > 80);
    let out = latest_activity_preview(&[bash_line(&long)]);
    assert_eq!(out.as_deref(), Some("Build step\nDeploy step"));
}

#[test]
fn preview_narration_beats_bash_comments() {
    // Priority 1 (narration) wins over priority 2 (bash comments).
    let out = latest_activity_preview(&[
        bash_line("# --- Installing deps ---\nnpm install"),
        FlowLine::Text("Wiring up the new module.".to_string()),
    ]);
    assert_eq!(out.as_deref(), Some("Wiring up the new module."));
}

// ── Mixed processing-log flow (tool calls + intermediate text) — #300 ──

#[test]
fn tool_plus_text_folds_into_one_blockquote() {
    let out = render_flow_html(
        &[
            tline("✅ bash", "git status"),
            FlowLine::Text("Checked the tree, all clean.".to_string()),
            tline("✅ read_file", "handler.rs"),
        ],
        None,
    );
    // ADR 0005 F1: tool + text fold into one blockquote body; the narration-led
    // status/count rides in the merged footer below the block.
    assert!(out.starts_with("<blockquote expandable><b>✅ bash</b> <code>git status</code>\n\n"));
    let footer = out.rsplit('\n').next().unwrap();
    assert_eq!(
        footer,
        "⚙️ Checked the tree, all clean. • 2 tool calls • ⏱ 0:00"
    );
    assert!(out.contains("<b>✅ bash</b> <code>git status</code>"));
    assert!(out.contains("Checked the tree, all clean."));
    assert!(out.contains("<b>✅ read_file</b> <code>handler.rs</code>"));
    assert!(!out.contains("<details>"));
}

#[test]
fn text_only_flow_uses_processing_log_header() {
    let out = render_flow_html(&[FlowLine::Text("Switching provider…".to_string())], None);
    // Text-only flow (0 tools) still has an entry, so it renders a block; the
    // footer log-summary shows the activity with a cog and no `N tool calls`.
    assert!(out.contains("<blockquote expandable>Switching provider…</blockquote>"));
    let footer = out.rsplit('\n').next().unwrap();
    assert_eq!(footer, "⚙️ Switching provider… • ⏱ 0:00");
    assert!(!out.contains("tool calls"));
}

#[test]
fn intermediate_text_is_html_escaped_in_flow() {
    let out = render_flow_html(
        &[
            tline("✅ bash", "echo hi"),
            FlowLine::Text("result: <b>bold</b> & <script>alert(1)</script>".to_string()),
        ],
        None,
    );
    assert!(out.contains("&lt;b&gt;bold&lt;/b&gt; &amp; &lt;script&gt;"));
    // The injected tags must never survive as live HTML.
    assert!(!out.contains("<script>"));
}

#[test]
fn blank_text_entries_are_dropped() {
    let out = render_flow_html(
        &[tline("✅ bash", "x"), FlowLine::Text("   ".to_string())],
        None,
    );
    // Blank text is dropped, leaving one tool entry rendered as a normal block.
    assert_eq!(
        out,
        "<blockquote expandable><b>✅ bash</b> <code>x</code></blockquote>\n\
         ⚙️ bash x • 1 tool calls • ⏱ 0:00"
    );
}

#[test]
fn empty_flow_renders_footer_only() {
    assert_eq!(render_flow_html(&[], None), "⏱ 0:00");
}

// ── folded_duplicates_final: block dedup against the final answer ──
// A streamed copy of the final answer can land folded in the collapsed block.
// On API providers the answer also comes back in response.content, so it must
// be dropped from the block or it renders twice (once folded, once as the
// completion). Streaming often folds only a truncated head, so the check must
// catch a PREFIX, not just an exact match.

#[test]
fn folded_dup_matches_exact_final() {
    let answer = "The rebuild finished and the binary is swapped in.";
    assert!(folded_duplicates_final(answer, answer));
}

#[test]
fn folded_dup_matches_truncated_prefix() {
    // The block captured only the streamed head, cut off mid-sentence.
    let folded = "Yes, Adolfo. After you told me to search for the tool, I";
    let final_text =
        "Yes, Adolfo. After you told me to search for the tool, I found it and used it.";
    assert!(folded_duplicates_final(folded, final_text));
}

#[test]
fn folded_dup_ignores_whitespace_differences() {
    let folded = "line one\n\n  line two three four five";
    let final_text = "line one line two three four five and the rest of the answer here";
    assert!(folded_duplicates_final(folded, final_text));
}

#[test]
fn folded_dup_rejects_distinct_narration() {
    // A genuine mid-turn narration line that isn't the final answer.
    let folded = "Let me trace the delivery path first.";
    let final_text = "The root cause is an exact-equality dedup that misses a truncated prefix.";
    assert!(!folded_duplicates_final(folded, final_text));
}

#[test]
fn folded_dup_rejects_short_shared_opening() {
    // Too short an overlap to be sure it's the answer, not a coincidental start.
    let folded = "Done.";
    let final_text = "Done. Here is the full breakdown of everything that changed this turn.";
    assert!(!folded_duplicates_final(folded, final_text));
}

#[test]
fn folded_dup_exact_short_answer_matches() {
    // #316: a short final answer folded verbatim used to slip under the
    // 20-char prefix guard and render twice (in the block AND as the
    // completion). Exact equality is a duplicate at any length.
    assert!(folded_duplicates_final("Dropped it.", "Dropped it."));
}

#[test]
fn folded_dup_exact_short_with_whitespace_matches() {
    assert!(folded_duplicates_final("  Dropped   it.\n", "Dropped it."));
}

#[test]
fn folded_dup_empty_sides_never_match() {
    assert!(!folded_duplicates_final("", ""));
    assert!(!folded_duplicates_final("", "Dropped it."));
    assert!(!folded_duplicates_final("Dropped it.", ""));
}

// ── Live status in the block header — single progress surface (#360) ──
// While the turn runs, the open block's header carries the rolling status
// ("N tool calls • read_file • 45s") so no standalone ticker message exists
// alongside the block. When the final response lands the status is cleared
// and the header settles back to the plain count.

#[test]
fn live_status_rides_in_footer() {
    // ADR 0005 F1: the live status/activity + count now rides in the merged
    // footer (plain line after the block), not a header inside the blockquote.
    // The wrapper passes elapsed_secs=0, so the clock reads 0:00 regardless of
    // the legacy live-status argument.
    let out = render_flow_html(
        &[
            tline("✅ bash", "cargo fmt"),
            FlowLine::Text("Reading the handler.".to_string()),
            tline("⚙️ read_file", "handler.rs"),
        ],
        Some("45s"),
    );
    assert!(out.starts_with("<blockquote expandable><b>✅ bash</b> <code>cargo fmt</code>\n\n"));
    let footer = out.rsplit('\n').next().unwrap();
    assert_eq!(footer, "⚙️ Reading the handler. • 2 tool calls • ⏱ 0:00");
}

#[test]
fn no_duration_still_leads_with_activity() {
    // No elapsed duration yet: the header still leads with the latest activity
    // (here the most-recent tool line fallback) and the count, with no trailing
    // duration segment (#509).
    let out = render_flow_html(
        &[
            tline("✅ bash", "cargo fmt"),
            tline("✅ read_file", "handler.rs"),
        ],
        None,
    );
    assert!(out.starts_with("<blockquote expandable><b>✅ bash</b> <code>cargo fmt</code>\n\n"));
    let footer = out.rsplit('\n').next().unwrap();
    assert_eq!(footer, "⚙️ read_file handler.rs • 2 tool calls • ⏱ 0:00");
    assert!(!out.contains("45s"));
}

#[test]
fn running_tool_fallback_does_not_double_the_gear() {
    // The bare-tool fallback leads with a still-running tool whose label already
    // carries the running gear (`⚙️ bash`). The header prints its own gear, so
    // without the strip this would render a double gear (#509 follow-up).
    let out = render_flow_html(
        &[
            tline("⚙️ bash", "gh repo view"),
            tline("⚙️ bash", "gh pr list"),
        ],
        Some("10s"),
    );
    assert!(
        !out.contains("⚙️ ⚙️"),
        "the running-tool fallback must not double the footer gear: {out}"
    );
    let footer = out.rsplit('\n').next().unwrap();
    assert_eq!(footer, "⚙️ bash gh pr list • 2 tool calls • ⏱ 0:00");
}

#[test]
fn live_status_on_text_only_flow_uses_processing_log_header() {
    let out = render_flow_html(
        &[FlowLine::Text("Looking into it.".to_string())],
        Some("15s"),
    );
    // Text-only flow: the narration + cog rides in the footer, no tool count.
    assert!(out.contains("<blockquote expandable>Looking into it.</blockquote>"));
    let footer = out.rsplit('\n').next().unwrap();
    assert_eq!(footer, "⚙️ Looking into it. • ⏱ 0:00");
}

#[test]
fn single_tool_progress_rides_in_footer() {
    // ADR 0005 F1: a lone tool call renders as a block; its progress (activity +
    // count) rides in the merged footer so the single surface shows progress
    // from call one.
    let out = render_flow_html(&[tline("⚙️ bash", "git status")], Some("bash • 5s"));
    assert_eq!(
        out,
        "<blockquote expandable><b>⚙️ bash</b> <code>git status</code></blockquote>\n\
         ⚙️ bash git status • 1 tool calls • ⏱ 0:00"
    );
    assert!(out.contains("<blockquote expandable>"));
}

// ── Header wall-clock duration + settled outcome states (#480) ──

#[test]
fn humanize_duration_precise_then_minutes() {
    // Precise wall-clock seconds under a minute, then "X min Ys" (#480).
    assert_eq!(humanize_duration(0), "0s");
    assert_eq!(humanize_duration(45), "45s");
    assert_eq!(humanize_duration(59), "59s");
    assert_eq!(humanize_duration(60), "1 min 0s");
    assert_eq!(humanize_duration(90), "1 min 30s");
    assert_eq!(humanize_duration(300), "5 min 0s");
    assert_eq!(humanize_duration(3661), "61 min 1s");
}

#[test]
fn flow_header_live_and_settled_formats() {
    // Live order (#509): gear + bold status message FIRST, then the italic
    // tool-call count, then the italic duration, `•`-separated.
    assert_eq!(
        flow_header_text(
            3,
            &FlowHeader::Live(Some("45s")),
            Some("Reading logs"),
            HeaderMarkup::Html
        ),
        "⚙️ <b>Reading logs</b> • <i>3 tool calls</i> • <i>45s</i>"
    );
    // No status message, duration present → count + duration (italic), no lead.
    assert_eq!(
        flow_header_text(3, &FlowHeader::Live(Some("45s")), None, HeaderMarkup::Html),
        "⚙️ <i>3 tool calls</i> • <i>45s</i>"
    );
    // Neither status nor duration (just started) → plain bold count / log.
    assert_eq!(
        flow_header_text(3, &FlowHeader::Live(None), None, HeaderMarkup::Html),
        "<b>3 tool calls</b>"
    );
    assert_eq!(
        flow_header_text(0, &FlowHeader::Live(None), None, HeaderMarkup::Html),
        "<b>Processing log</b>"
    );
    // Markdown dialect swaps the tags: `**`/`_` where HTML uses `<b>`/`<i>`.
    assert_eq!(
        flow_header_text(
            3,
            &FlowHeader::Live(Some("45s")),
            Some("Reading logs"),
            HeaderMarkup::Markdown
        ),
        "⚙️ **Reading logs** • _3 tool calls_ • _45s_"
    );
    // Live header with completed/failed tool status glyphs strips the glyph
    // so in-flight turns keep the ⚙️ gear and do NOT present bare ✅ / ❌.
    assert_eq!(
        flow_header_text(
            55,
            &FlowHeader::Live(Some("3:14")),
            Some("✅ glob reviews/20260915-c14/**"),
            HeaderMarkup::Markdown
        ),
        "⚙️ **glob reviews/20260915-c14/**** • _55 tool calls_ • _3:14_"
    );
    assert_eq!(
        flow_header_text(
            55,
            &FlowHeader::Live(Some("3:14")),
            Some("❌ read_file missing.md"),
            HeaderMarkup::Html
        ),
        "⚙️ <b>read_file missing.md</b> • <i>55 tool calls</i> • <i>3:14</i>"
    );
    // Live header with genuine system status icons (like ⏳ compaction) keeps
    // the icon bare and drops the standing gear.
    assert_eq!(
        flow_header_text(
            2,
            &FlowHeader::Live(Some("10s")),
            Some("⏳ Compacting context…"),
            HeaderMarkup::Html
        ),
        "<b>⏳ Compacting context…</b> • <i>2 tool calls</i> • <i>10s</i>"
    );
    // Settled: outcome verb, count-in-parens, duration, fully bold; count clause
    // dropped when no tools ran. The status-message arg is ignored when settled.
    assert_eq!(
        flow_header_text(
            12,
            &FlowHeader::Settled {
                icon: "✅",
                verb: "Finished",
                duration: "45s"
            },
            None,
            HeaderMarkup::Html
        ),
        "<b>✅ Finished (12 tool calls, 45s)</b>"
    );
    assert_eq!(
        flow_header_text(
            0,
            &FlowHeader::Settled {
                icon: "✅",
                verb: "Finished",
                duration: "45s"
            },
            None,
            HeaderMarkup::Html
        ),
        "<b>✅ Finished (45s)</b>"
    );
}

#[test]
fn flow_outcome_icons_and_verbs() {
    assert_eq!(FlowOutcome::Finished.icon_verb(), ("✅", "Finished"));
    assert_eq!(FlowOutcome::Failed.icon_verb(), ("❌", "Failed"));
    assert_eq!(FlowOutcome::TimedOut.icon_verb(), ("⏱", "Timed out"));
}

#[test]
fn settled_outcome_renders_block_header_over_lone_tool() {
    // A settled outcome always renders the block header (not the live lone-tool
    // one-liner) so the ❌/✅/⏱ badge, count, and duration show (#480).
    let out = render_flow_html_with(
        &[tline("✅ bash", "cargo test")],
        &FlowHeader::Settled {
            icon: "❌",
            verb: "Failed",
            duration: "12s",
        },
    );
    // ADR 0005 F1: the settled outcome badge + count now renders in the merged
    // footer below the log block, not as a header inside the blockquote.
    assert!(
        out.starts_with(
            "<blockquote expandable><b>✅ bash</b> <code>cargo test</code></blockquote>"
        )
    );
    let footer = out.rsplit('\n').next().unwrap();
    assert_eq!(
        footer, "❌ Failed • 1 tool calls • ⏱ 0:00",
        "settled footer: {out}"
    );
}

#[test]
fn settled_block_carries_no_activity_preview_classic() {
    // #498: the latest-activity preview is live-only. Once the block settles,
    // the stale narration must NOT stick under the Finished header — the rollup
    // stands alone. The narration still lives in the collapsed body log; only
    // the header-attached `•` preview is suppressed.
    let out = render_flow_html_with(
        &[
            tline("✅ bash", "cargo test"),
            FlowLine::Text("Running the test suite".to_string()),
        ],
        &FlowHeader::Settled {
            icon: "✅",
            verb: "Finished",
            duration: "3 min 15s",
        },
    );
    // The narration still lives in the collapsed body log, but the settled
    // footer shows only the outcome + bare count — no cog, no stale activity.
    let footer = out.rsplit('\n').next().unwrap();
    assert_eq!(footer, "✅ Finished • 1 tool calls • ⏱ 0:00");
    assert!(
        !footer.contains("Running the test suite"),
        "settled footer must not carry a preview: {out}"
    );
    assert!(
        !footer.contains("⚙️"),
        "settled footer carries no cog: {out}"
    );
    assert!(out.contains("Running the test suite"));
}

#[test]
fn settled_block_carries_no_activity_preview_rich() {
    // #498, rich <details> surface (the one Adolfo runs): settled summary is
    // the outcome header alone, no trailing `•` preview stuck to it.
    let out = render_flow_details_with(
        &[
            tline("✅ bash", "cargo test"),
            FlowLine::Text("Running the test suite".to_string()),
        ],
        &FlowHeader::Settled {
            icon: "✅",
            verb: "Finished",
            duration: "3 min 15s",
        },
    );
    let summary_end = out.find("</summary>").expect("summary");
    let summary = &out[..summary_end];
    assert!(summary.contains("✅ Finished • 1 tool calls"));
    assert!(
        !summary.contains("Running the test suite"),
        "settled summary must not carry a preview: {summary}"
    );
    assert!(
        !summary.contains("⚙️"),
        "settled summary carries no cog: {summary}"
    );
}

// ── Rich API flow rendering tests (#393) ─────────────────────────────────────

#[test]
fn rich_empty_group_renders_header_only() {
    assert_eq!(render_flow_rich(&[], None, false), "**Processing log**");
}

#[test]
fn rich_single_tool_renders_plain_line() {
    let out = render_flow_rich(&[tline("✅ bash", "git status")], None, false);
    assert_eq!(out, "**✅ bash** `git status`");
    // Single tool: no blockquote wrapping, just bold label + context
    assert!(!out.contains(">"));
}

#[test]
fn rich_multiple_tools_render_markdown_header() {
    let out = render_flow_rich(
        &[tline("✅ bash", "git status"), tline("✅ read", "file.rs")],
        None,
        false,
    );
    // No narration: the activity fallback (most recent tool line) leads the
    // header, bold, then the italic count (#509). Icon-led activity: the
    // standing gear is stripped (owner directive 2026-08-29).
    assert!(out.starts_with("⚙️ **read file.rs** • _2 tool calls_\n\n"));
    assert!(out.contains("**✅ bash** `git status`"));
    assert!(out.contains("**✅ read** `file.rs`"));
}

#[test]
fn rich_live_status_in_header() {
    // Status-first order (#509): narration (bold) leads, then the count and the
    // duration (italic).
    let out = render_flow_rich(
        &[
            tline("✅ bash", "x"),
            FlowLine::Text("Searching.".to_string()),
            tline("⚙️ grep", "pattern"),
        ],
        Some("10s"),
        false,
    );
    assert!(out.starts_with("⚙️ **Searching.** • _2 tool calls_ • _10s_\n\n"));
}

#[test]
fn rich_single_tool_live_status_appends() {
    let out = render_flow_rich(&[tline("⚙️ bash", "git status")], Some("bash • 5s"), false);
    assert_eq!(out, "**⚙️ bash** `git status` • bash • 5s");
}

// ── Rich-API details flow rendering (#420 path A) ──

#[test]
fn details_empty_group_renders_footer_only() {
    // ADR 0005 F1: no entries → a plain <sub> footer line, no <details> wrapper.
    assert_eq!(render_flow_details(&[], None), "<sub>⏱ 0:00</sub>");
}

#[test]
fn details_single_tool_renders_details_block() {
    // ADR 0005 F1: a lone tool line now renders as a full <details> block, the
    // merged footer as the <sub> summary and the entry as a <p> body.
    let out = render_flow_details(&[tline("✅ bash", "git status")], None);
    assert_eq!(
        out,
        "<details><summary><sub>⚙️ bash git status • 1 tool calls • ⏱ 0:00</sub></summary>\
         <p><b>✅ bash</b> <code>git status</code></p></details>"
    );
    assert!(out.contains("<details>"));
}

#[test]
fn details_multiple_tools_wrap_in_collapsed_details() {
    let out = render_flow_details(
        &[tline("✅ bash", "git status"), tline("✅ read", "file.rs")],
        None,
    );
    // Collapsed by default: plain <details>, never <details open>. The merged
    // footer is the <sub> summary and carries the latest-activity preview so the
    // collapsed block shows progress with the body hidden (#405); with no
    // narration it falls back to the most recent tool line.
    assert!(out.starts_with(
        "<details><summary><sub>⚙️ read file.rs • 2 tool calls • ⏱ 0:00</sub></summary>"
    ));
    assert!(out.ends_with("</details>"));
    assert!(!out.contains("<details open"));
    // Each entry is its own <p>: the rich HTML parser ignores raw newlines,
    // so without block-level wrapping the log runs together as one wall.
    assert!(out.contains("<p><b>✅ bash</b> <code>git status</code></p>"));
    assert!(out.contains("<p><b>✅ read</b> <code>file.rs</code></p>"));
}

#[test]
fn details_summary_carries_live_status() {
    let out = render_flow_details(
        &[
            tline("✅ bash", "x"),
            FlowLine::Text("Grepping.".to_string()),
            tline("⚙️ grep", "pattern"),
        ],
        Some("10s"),
    );
    let summary_end = out.find("</summary>").expect("summary");
    let summary = &out[..summary_end];
    // ADR 0005 F1: the merged footer is the summary — cog + narration + count +
    // clock. The wrapper passes elapsed_secs=0, so the clock reads 0:00.
    assert!(summary.contains("⚙️ Grepping. • 2 tool calls • ⏱ 0:00"));
    // Summary is wrapped in <sub> for visual de-emphasis (#436).
    assert!(summary.contains("<sub>⚙️ Grepping."));
    // Latest-activity preview rides in the summary (#405): the rich <details>
    // collapses to the summary ALONE, hiding the body, so without the preview
    // the collapsed block shows no progress at all.
    assert!(summary.contains("Grepping."));
}

#[test]
fn details_collapsed_summary_shows_intermediate_narration() {
    // Regression (#405): the COLLAPSED rich block must surface the latest
    // narration in its summary — the body is hidden by default, so the summary
    // is the only place a user sees mid-turn progress. Narration wins over the
    // trailing tool line as the preview source (#481).
    let out = render_flow_details(
        &[
            FlowLine::Text("Running the test suite".to_string()),
            tline("⚙️ bash", "cargo test"),
        ],
        Some("45s"),
    );
    let summary_end = out.find("</summary>").expect("summary");
    let summary = &out[..summary_end];
    // ADR 0005 F1: narration leads the merged-footer summary after the cog.
    assert!(summary.contains("⚙️ Running the test suite"));
}

#[test]
fn details_escapes_html_in_tool_context() {
    let out = render_flow_details(&[tline("✅ bash", "a < b"), tline("✅ read", "x.rs")], None);
    assert!(out.contains("<code>a &lt; b</code>"));
}

// ── Latest-activity preview in the collapsed block (#405) ──

#[test]
fn collapsed_preview_prefers_narration_over_tool_line() {
    // #481: the status source is the most recent human-readable narration — the
    // latest thing the agent SAID — even when a tool line follows it (that
    // narration usually describes the tool now running), which reads better
    // than a bare "⚙️ read_file src/agent.rs".
    let out = render_flow_html(
        &[
            FlowLine::Text("Checking how the scheduler resolves the next run".to_string()),
            tline("✅ bash", "grep flow"),
            tline("⚙️ read_file", "src/agent.rs"),
        ],
        None,
    );
    // ADR 0005 F1: the narration preview now rides in the merged footer, not a
    // header line above the entries.
    let footer = out.rsplit('\n').next().unwrap();
    assert!(
        footer.contains("Checking how the scheduler resolves the next run"),
        "footer must carry the narration preview: {footer}"
    );
    // Full chronological log still follows for the expanded view.
    assert!(out.contains("<b>⚙️ read_file</b> <code>src/agent.rs</code>"));
}

#[test]
fn preview_keeps_long_text_whole_and_strips_markdown() {
    // #481 amendment: no truncation. Inline markers still stripped so the
    // preview never shows raw ** source.
    let long = format!("**{}**", "x".repeat(200));
    let out = render_flow_html(&[tline("✅ bash", "a"), FlowLine::Text(long)], None);
    // ADR 0005 F1: the narration preview rides in the merged footer; it stays
    // whole and markdown markers are stripped before it is shown.
    let footer = out.rsplit('\n').next().unwrap();
    assert!(!footer.contains('…'), "not truncated: {footer}");
    assert!(
        footer.contains(&"x".repeat(200)),
        "whole text kept: {footer}"
    );
    assert!(!footer.contains("**"), "markers stripped: {footer}");
}

// ── trailing folded-answer reclaim (#478) ───────────────────────────

#[test]
fn trailing_text_run_pops_joined_in_order() {
    // The incident shape: tool rounds, then TWO trailing answer texts
    // (the second folded after a queued follow-up). Both must come out,
    // in order, so neither stays imprisoned in the block.
    let mut entries = vec![
        FlowEntry::Tool(0),
        FlowEntry::Text("first part of the answer".into()),
        FlowEntry::Text("Already factored in. Standing by.".into()),
    ];
    let (reclaimed, trailer) = pop_trailing_folded_texts(&mut entries, false);
    assert_eq!(
        reclaimed.as_deref(),
        Some("first part of the answer\n\nAlready factored in. Standing by.")
    );
    assert_eq!(trailer, None);
    assert_eq!(entries.len(), 1, "tool entry stays");
    assert!(matches!(entries[0], FlowEntry::Tool(0)));
}

#[test]
fn text_only_flow_pops_everything() {
    let mut entries = vec![
        FlowEntry::Text("the whole answer".into()),
        FlowEntry::Text("plus a follow-up".into()),
    ];
    let (reclaimed, _) = pop_trailing_folded_texts(&mut entries, false);
    assert!(
        reclaimed
            .as_deref()
            .unwrap_or_default()
            .starts_with("the whole answer")
    );
    assert!(entries.is_empty());
}

#[test]
fn tool_last_flow_reclaims_nothing() {
    // Trailing tool call = the answer is in response.content, not folded.
    let mut entries = vec![FlowEntry::Text("narration".into()), FlowEntry::Tool(0)];
    assert_eq!(pop_trailing_folded_texts(&mut entries, false), (None, None));
    assert_eq!(entries.len(), 2, "nothing consumed");
}

// ── folded narration cap keeps the block compact (#489) ─────────────

#[test]
fn long_folded_narration_is_capped_in_the_block() {
    // A verbose CLI narration entry (over the 300-char cap) renders
    // truncated with an ellipsis, so the collapsed block stays small and
    // more tool rounds fit before the 30K rich size freeze.
    let long = "x".repeat(1200);
    let out = render_flow_html(
        &[
            tline("✅ bash", "cargo test"),
            FlowLine::Text(long.clone()),
            FlowLine::Text("all done here now".into()),
            tline("✅ read", "flow.rs"),
        ],
        None,
    );
    assert!(out.contains('…'), "capped body entry ends with an ellipsis");
    // A LATER human text is the latest-activity preview (#487, shown whole),
    // so the long entry is body-only and its full run must NOT appear.
    assert!(!out.contains(&long), "body narration must be truncated");
}
