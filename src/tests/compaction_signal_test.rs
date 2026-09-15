//! Compaction signal (#29): unit tests for the flow-line builders, the
//! pinned-header const, the ETA predictor, and the footer-dedupe contract.
//! All pure — no agent, no mocks, no locks.

use std::time::Duration;

use crate::brain::agent::service::nudge::{in_pressure_warning_band, should_emit_pressure_warning};
use crate::channels::telegram::flow::{
    COMPACTING_HEADER_TEXT, FlowHeader, FlowLine, HeaderMarkup, compacted_flow_line,
    compacting_flow_line, flow_header_text, render_flow_html_chrome_pref, render_flow_rich,
    starts_with_icon,
};
use crate::channels::telegram::flow_chrome::{FlowSections, FooterParts, merged_footer};

#[test]
fn compacting_line_without_prediction() {
    // First compaction of a session: no observed history, no parenthetical.
    // The retired hardcoded `≈10–60s` window is gone — it never was right
    // (live range observed 10s → 29 min).
    assert_eq!(
        compacting_flow_line(68.0, None),
        "⏳ Compacting context — 68% full…"
    );
}

#[test]
fn compacting_line_shows_observed_eta() {
    assert_eq!(
        compacting_flow_line(68.0, Some(Duration::from_secs(42))),
        "⏳ Compacting context — 68% full (≈42s)…"
    );
}

#[test]
fn compacting_line_humanizes_minute_eta() {
    // ≥60s rides the shared humanize_duration formatting ("2 min 12s"),
    // same as the settled ✅ line.
    assert_eq!(
        compacting_flow_line(71.0, Some(Duration::from_secs(132))),
        "⏳ Compacting context — 71% full (≈2 min 12s)…"
    );
}

#[test]
fn compacting_line_rounds_fill_level() {
    // {:.0} rounding — the line carries a whole-number level, never a
    // fractional one.
    assert!(compacting_flow_line(67.7, None).contains("68% full"));
    assert!(compacting_flow_line(99.4, None).contains("99% full"));
}

#[test]
fn compacting_footer_suppresses_duplicate_activity_segment() {
    // Dedupe (#29): during the silent window the newest log line IS the ⏳
    // body entry, so the activity preview renders the compaction string a
    // second time next to the pinned header — the owner-sighted duplication.
    // While compacting the flag suppresses the activity segment: exactly ONE
    // compaction string. With the flag off the duplicate returns (documents
    // the suppression contract).
    let lines = [FlowLine::Text(
        "⏳ Compacting context — 66% full…".to_string(),
    )];
    let compacting = render_flow_html_chrome_pref(
        &lines,
        &FlowHeader::Live(Some(COMPACTING_HEADER_TEXT)),
        None,
        &FlowSections::default(),
        usize::MAX,
        5,
        None,
        true,
    );
    assert_eq!(
        compacting.matches("Compacting context").count(),
        1,
        "pinned header is the sole compaction string while compacting: {compacting:?}"
    );
    let idle = render_flow_html_chrome_pref(
        &lines,
        &FlowHeader::Live(Some("⚙️")),
        None,
        &FlowSections::default(),
        usize::MAX,
        5,
        None,
        false,
    );
    assert_eq!(
        idle.matches("Compacting context").count(),
        2,
        "without the flag the activity preview duplicates the body entry: {idle:?}"
    );
}

#[test]
fn compacting_rich_footer_suppresses_duplicate_status() {
    let lines = [FlowLine::Text(
        "⏳ Compacting context — 66% full…".to_string(),
    )];
    // Flag on: the HEADER (first line) carries the pinned compaction string
    // exactly once — the activity preview is suppressed. The body keeps the
    // ⏳ START line (a real log entry, the fill-level carrier), so the count
    // is asserted on the header line, never the whole message.
    let rich = render_flow_rich(&lines, Some(COMPACTING_HEADER_TEXT), true);
    let header = rich.lines().next().unwrap_or_default();
    assert_eq!(
        header.matches("Compacting context").count(),
        1,
        "rich header dedupes too: {rich:?}"
    );
    assert!(
        rich.contains("⏳ Compacting context — 66% full…"),
        "body keeps the START line: {rich:?}"
    );
    // Flag off: the header duplicates the body entry (documents the
    // suppression contract).
    let idle = render_flow_rich(&lines, Some(COMPACTING_HEADER_TEXT), false);
    let idle_header = idle.lines().next().unwrap_or_default();
    assert_eq!(
        idle_header.matches("Compacting context").count(),
        2,
        "without the flag the activity preview duplicates the pin: {idle:?}"
    );
}

#[test]
fn compacted_line_under_a_minute() {
    assert_eq!(
        compacted_flow_line(68.0, 26.0, Duration::from_secs(42)),
        "✅ Compacted: 68% → 26% in 42s"
    );
}

#[test]
fn compacted_line_multi_minute() {
    assert_eq!(
        compacted_flow_line(71.0, 24.0, Duration::from_secs(132)),
        "✅ Compacted: 71% → 24% in 2 min 12s"
    );
}

#[test]
fn compacted_line_floors_subsecond_elapsed_to_1s() {
    // A sub-second summarizer call still reads as a real duration — "0s"
    // would look like the line was printed before the work happened.
    assert_eq!(
        compacted_flow_line(66.0, 30.0, Duration::from_millis(300)),
        "✅ Compacted: 66% → 30% in 1s"
    );
}

#[test]
fn header_const_is_number_free() {
    // Design invariant (#29): the pinned header NEVER carries a number —
    // compaction progress is unknowable, so any digit reads as a fake
    // progress bar. The fill level lives on the START body line instead.
    assert_eq!(COMPACTING_HEADER_TEXT, "⏳ Compacting context…");
    assert!(!COMPACTING_HEADER_TEXT.contains('%'));
    assert!(!COMPACTING_HEADER_TEXT.chars().any(|c| c.is_ascii_digit()));
}

#[test]
fn pressure_band_boundaries() {
    // [55, 65) — the ceiling is exclusive: AT 65% compaction itself fires,
    // the nudge is for the approach.
    assert!(!in_pressure_warning_band(54.9));
    assert!(in_pressure_warning_band(55.0));
    assert!(in_pressure_warning_band(64.9));
    assert!(!in_pressure_warning_band(65.0));
}

#[test]
fn pressure_warning_once_per_entry() {
    // In-band, not yet emitted → warn; already emitted → silence until the
    // flag re-arms below the floor; below the floor → never warn.
    assert!(should_emit_pressure_warning(60.0, false).is_some());
    assert!(should_emit_pressure_warning(60.0, true).is_none());
    assert!(should_emit_pressure_warning(40.0, false).is_none());
}

// ── gear-strip (#29 fix round, owner directive): the standing ⚙️ chrome
// prefix is dropped whenever another icon follows it ──

#[test]
fn starts_with_icon_classification() {
    // Icon glyphs drop the standing gear; word text (Latin, Cyrillic) keeps it.
    assert!(starts_with_icon("⏳ Compacting context…"));
    assert!(starts_with_icon("✅ bash git status"));
    assert!(starts_with_icon("❌ grep pattern"));
    assert!(!starts_with_icon("bash gh pr list"));
    assert!(!starts_with_icon("Reading the handler."));
    assert!(!starts_with_icon("Чтение лога"));
    assert!(!starts_with_icon(""));
}

#[test]
fn live_header_drops_gear_before_icon_status() {
    // The pinned compaction status leads with ⏳ → the header renders bare.
    assert_eq!(
        flow_header_text(
            11,
            &FlowHeader::Live(Some("1:05")),
            Some(COMPACTING_HEADER_TEXT),
            HeaderMarkup::Html
        ),
        "<b>⏳ Compacting context…</b> • <i>11 tool calls</i> • <i>1:05</i>"
    );
    // Plain status keeps the gear (#509 shape unchanged).
    assert_eq!(
        flow_header_text(
            3,
            &FlowHeader::Live(Some("0:12")),
            Some("Reading logs"),
            HeaderMarkup::Html
        ),
        "⚙️ <b>Reading logs</b> • <i>3 tool calls</i> • <i>0:12</i>"
    );
}

#[test]
fn live_footer_drops_gear_before_icon_activity() {
    // In-flight activity strips tool outcome glyphs (✅/❌) so unsettled turns
    // maintain the running ⚙️ cog, while system pins like ⏳ remain bare (#1052).
    let tool_done = merged_footer(
        &FooterParts {
            outcome: None,
            plan_state: None,
            working_on: None,
            activity: Some("✅ bash git status"),
            tool_count: 1,
            has_log: true,
            ctx: None,
            elapsed_secs: 0,
            bg: None,
        },
        HeaderMarkup::Markdown,
    );
    assert_eq!(tool_done, "⚙️ bash git status • 1 tool calls • ⏱ 0:00");
    let icon = merged_footer(
        &FooterParts {
            outcome: None,
            plan_state: None,
            working_on: None,
            activity: Some("⏳ Compacting context…"),
            tool_count: 1,
            has_log: true,
            ctx: None,
            elapsed_secs: 0,
            bg: None,
        },
        HeaderMarkup::Markdown,
    );
    assert_eq!(icon, "⏳ Compacting context… • 1 tool calls • ⏱ 0:00");
    let plain = merged_footer(
        &FooterParts {
            outcome: None,
            plan_state: None,
            working_on: None,
            activity: Some("Reading the handler."),
            tool_count: 2,
            has_log: true,
            ctx: None,
            elapsed_secs: 0,
            bg: None,
        },
        HeaderMarkup::Markdown,
    );
    assert_eq!(plain, "⚙️ Reading the handler. • 2 tool calls • ⏱ 0:00");
}

#[test]
fn icon_led_segment_retires_the_bare_cog_fallback() {
    // With an icon-led pin as the only narration (activity suppressed by the
    // compaction dedupe, zero tool calls), the bare-⚙️ fallback segment is
    // redundant — the icon already signals activity.
    let out = merged_footer(
        &FooterParts {
            outcome: None,
            plan_state: None,
            working_on: Some(COMPACTING_HEADER_TEXT),
            activity: None,
            tool_count: 0,
            has_log: true,
            ctx: None,
            elapsed_secs: 65,
            bg: None,
        },
        HeaderMarkup::Markdown,
    );
    assert_eq!(out, "⏳ Compacting context… • ⏱ 1:05");
}
