//! Tests for the Mission Control analytics panel renderer (#900).
//!
//! Renders the real `analytics_panel::render` into a ratatui `TestBackend`
//! buffer and asserts on the produced cells, plus drives the pure `decide`
//! fn to verify the global D/W/M/A filter switching and scroll behaviour that
//! feed the renderer. Covers: the pinned filter tabs (active tab bold), the
//! recovery / verify counters in Totals, the per-model reliability rows with
//! their ✓/⚠/✗ status glyphs, the phantom card, the window label tracking the
//! active filter, empty-section hiding, the responsive multi-card grid, and no
//! layout regression at 80-col and 120-col.

use crate::brain::mission_control::types::{
    McAnalytics, McBrainFile, McBrainVerifyStats, McModelToolStat, McPhantomStats,
    McStreamingStats, McToolStat, TimeWindow,
};
use crate::tui::app::mission_control::McPanel;
use crate::tui::app::mission_control::McState;
use crate::tui::app::mission_control::input::{KeyOutcome, decide};
use crate::tui::render::mission_control::render_analytics;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

/// Render the analytics panel into a fresh buffer and return it.
fn render_buf(a: &McAnalytics, window: TimeWindow, scroll: u16, w: u16, h: u16) -> Buffer {
    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).unwrap();
    let area = Rect::new(0, 0, w, h);
    terminal
        .draw(|f| render_analytics(f, a, window, scroll, area, true))
        .unwrap();
    terminal.backend().buffer().clone()
}

/// Whole buffer flattened to one string (rows joined by '\n') for `.contains`.
fn buf_text(buf: &Buffer) -> String {
    let area = buf.area;
    (0..area.height)
        .map(|row| row_text(buf, row, area.width))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Extract the text of a single buffer row.
fn row_text(buf: &Buffer, row: u16, width: u16) -> String {
    (0..width)
        .map(|col| {
            buf.cell((col, row))
                .map(|c| c.symbol().to_string())
                .unwrap_or_default()
        })
        .collect::<String>()
}

/// A fixture with every section populated so render exercises all builders.
fn full_fixture() -> McAnalytics {
    McAnalytics {
        tool_total_calls: 1000,
        tool_total_fails: 50,
        top_tools: vec![McToolStat {
            name: "bash".into(),
            total: 500,
            failures: 10,
            fail_rate: 2.0,
        }],
        flakiest_tools: vec![McToolStat {
            name: "flaky_tool".into(),
            total: 10,
            failures: 5,
            fail_rate: 50.0,
        }],
        rsi_applied_total: 12,
        rsi_top_dimensions: vec![("voice".into(), 4)],
        brain_files: vec![McBrainFile {
            name: "MEMORY.md".into(),
            kb: 8.0,
        }],
        brain_total_kb: 8.0,
        phantom: McPhantomStats {
            total: 20,
            resolved: 15,
            resolved_pct: 75.0,
            by_model: vec![("claude-opus-4".into(), 10, 8)],
        },
        streaming: McStreamingStats {
            total: 7,
            ..Default::default()
        },
        brain_verify: McBrainVerifyStats {
            passes: 42,
            rollbacks: 3,
            ..Default::default()
        },
        model_tools: vec![McModelToolStat {
            model: "claude-opus-4".into(),
            total: 100,
            failures: 5,
            fail_rate: 5.0,
        }],
        ..Default::default()
    }
}

// ── Filter tabs ─────────────────────────────────────────────────────────────

#[test]
fn tabs_render_all_four_windows_with_hotkey_hint() {
    let buf = render_buf(&McAnalytics::default(), TimeWindow::Month, 0, 80, 24);
    let text = buf_text(&buf);
    assert!(text.contains("[D]"), "missing Day tab");
    assert!(text.contains("[W]"), "missing Week tab");
    assert!(text.contains("[M]"), "missing Month tab");
    assert!(text.contains("[All]"), "missing All tab");
    // #1775 moved the switch hint out of the panel and into the shared
    // Mission Control footer table, so the discoverability proof is the
    // footer's D/W/M/A binding row, not a trailing string in the panel.
    assert!(
        crate::tui::app::dialog_keys::dialog_keys(
            crate::tui::app::dialog_keys::DialogScope::MissionControl
        )
        .iter()
        .any(|key| key.label == "D/W/M/A"),
        "the Mission Control footer must advertise the window switch hotkeys"
    );
}

#[test]
fn active_tab_is_bold_others_are_not() {
    // Month active: the [M] cells carry BOLD, the [D] cells do not. Scan only
    // the tab region (cols 0..18) so the literal D/M letters in the trailing
    // "D/W/M/A to switch" hint, which are dim, can't poison the check.
    let buf = render_buf(&McAnalytics::default(), TimeWindow::Month, 0, 80, 24);
    let mut d_bold = false;
    let mut m_bold = false;
    for col in 0..18u16 {
        if let Some(cell) = buf.cell((col, 1)) {
            let bold = cell.style().add_modifier.contains(Modifier::BOLD);
            match cell.symbol() {
                "D" => d_bold = bold,
                "M" => m_bold = bold,
                _ => {}
            }
        }
    }
    assert!(m_bold, "active Month tab should be bold");
    assert!(!d_bold, "inactive Day tab should not be bold");
}

// ── Totals: recovery + verify counters ──────────────────────────────────────

#[test]
fn totals_show_recovery_and_verify_counters() {
    let a = McAnalytics {
        tool_total_calls: 100,
        tool_total_fails: 5,
        streaming: McStreamingStats {
            total: 7,
            ..Default::default()
        },
        brain_verify: McBrainVerifyStats {
            passes: 42,
            rollbacks: 3,
            ..Default::default()
        },
        ..Default::default()
    };
    let text = buf_text(&render_buf(&a, TimeWindow::Month, 0, 80, 30));
    assert!(text.contains("Totals"), "missing Totals card title");
    assert!(text.contains("100 calls"), "missing tool call total");
    assert!(text.contains("5 (5.0%)"), "missing fail count/rate");
    assert!(text.contains("7 recovered"), "missing recovery counter");
    assert!(text.contains("42 ok / 3 rollbk"), "missing verify counter");
}

// ── Model comparison + status glyphs ────────────────────────────────────────

#[test]
fn model_comparison_row_shows_fail_and_phantom_rate() {
    let a = McAnalytics {
        model_tools: vec![McModelToolStat {
            model: "claude-opus-4".into(),
            total: 100,
            failures: 5,
            fail_rate: 5.0,
        }],
        phantom: McPhantomStats {
            total: 10,
            resolved: 8,
            resolved_pct: 80.0,
            by_model: vec![("claude-opus-4".into(), 10, 8)],
        },
        ..Default::default()
    };
    let text = buf_text(&render_buf(&a, TimeWindow::Month, 0, 80, 30));
    assert!(
        text.contains("Model reliability"),
        "missing model card title"
    );
    assert!(text.contains("claude-opus-4"), "missing model name");
    assert!(text.contains("fail 5.0%"), "missing fail rate");
    // phantom_rate = 10 phantoms / 100 calls = 10.0% -> bad (>=10%) -> ✗ glyph.
    assert!(text.contains("ph 10.0%"), "missing phantom rate");
    assert!(
        text.contains("✗"),
        "phantom-heavy model should carry the ✗ glyph"
    );
}

#[test]
fn healthy_model_row_shows_check_glyph() {
    // Low fail rate and zero phantoms -> healthy -> ✓ glyph.
    let a = McAnalytics {
        model_tools: vec![McModelToolStat {
            model: "gpt-5".into(),
            total: 200,
            failures: 2,
            fail_rate: 1.0,
        }],
        ..Default::default()
    };
    let text = buf_text(&render_buf(&a, TimeWindow::Month, 0, 80, 30));
    assert!(text.contains("gpt-5"), "missing model name");
    assert!(text.contains("✓"), "healthy model should carry the ✓ glyph");
    assert!(
        !text.contains("✗"),
        "healthy model must not carry the ✗ glyph"
    );
}

// ── Phantom card ────────────────────────────────────────────────────────────

#[test]
fn phantom_panel_renders_detected_resolved_and_per_model() {
    let a = McAnalytics {
        phantom: McPhantomStats {
            total: 20,
            resolved: 15,
            resolved_pct: 75.0,
            by_model: vec![("gpt-5".into(), 12, 9)],
        },
        ..Default::default()
    };
    let text = buf_text(&render_buf(&a, TimeWindow::Month, 0, 80, 30));
    assert!(text.contains("Phantoms"), "missing Phantoms card title");
    assert!(
        text.contains("20 detected, 15 resolved (75.0%)"),
        "missing detected/resolved line"
    );
    assert!(text.contains("gpt-5"), "missing per-model name");
    assert!(text.contains("12 (9 resolved)"), "missing per-model counts");
}

// ── D/W/M label tracks the active window ────────────────────────────────────

#[test]
fn flakiest_card_title_tracks_active_window() {
    let a = McAnalytics {
        flakiest_tools: vec![McToolStat {
            name: "flaky_tool".into(),
            total: 10,
            failures: 5,
            fail_rate: 50.0,
        }],
        ..Default::default()
    };
    let cases = [
        (TimeWindow::Day, "24h"),
        (TimeWindow::Week, "7d"),
        (TimeWindow::Month, "30d"),
        (TimeWindow::All, "all-time"),
    ];
    for (window, label) in cases {
        let text = buf_text(&render_buf(&a, window, 0, 80, 30));
        assert!(text.contains("Flakiest"), "missing Flakiest card title");
        assert!(
            text.contains(label),
            "window {window:?} should label flakiest as {label}"
        );
    }
}

// ── Empty sections are hidden ───────────────────────────────────────────────

#[test]
fn empty_sections_are_hidden_but_totals_and_tabs_remain() {
    let text = buf_text(&render_buf(
        &McAnalytics::default(),
        TimeWindow::Month,
        0,
        80,
        40,
    ));
    // Totals card + tabs always render.
    assert!(text.contains("Totals"), "Totals card should always render");
    assert!(text.contains("[M]"), "tabs should always render");
    // Empty sections must not render their card titles.
    for absent in [
        "Model reliability",
        "Phantoms",
        "Flakiest",
        "Top tools",
        "Brain files",
        "RSI applied",
    ] {
        assert!(
            !text.contains(absent),
            "{absent} card should be hidden when empty"
        );
    }
}

// ── Responsive multi-card grid ──────────────────────────────────────────────

#[test]
fn populated_sections_render_as_distinct_cards() {
    // Full fixture populates every section; at 120-col the grid is 3-across,
    // so all card titles render in their own bordered boxes.
    let text = buf_text(&render_buf(&full_fixture(), TimeWindow::Month, 0, 120, 30));
    for title in [
        "Totals",
        "Model reliability",
        "Phantoms",
        "Top tools",
        "Brain files",
        "RSI applied",
    ] {
        assert!(text.contains(title), "missing {title} card");
    }
    // Flakiest carries the window label in its title.
    assert!(
        text.contains("Flakiest (30d)"),
        "missing labelled Flakiest card"
    );
}

// ── No layout regression at 80-col and 120-col ──────────────────────────────

#[test]
fn renders_at_80_and_120_col_without_panic() {
    let a = full_fixture();
    for width in [80u16, 120] {
        let buf = render_buf(&a, TimeWindow::Month, 0, width, 24);
        let text = buf_text(&buf);
        assert!(text.contains("[All]"), "tabs should render at {width}-col");
        assert!(
            text.contains("Totals"),
            "Totals card should render at {width}-col"
        );
    }
}

// ── decide(): global D/W/M/A filter switching + scroll ──────────────────────

#[test]
fn letter_keys_switch_window_from_any_panel() {
    // Global: works even when a non-Analytics panel is focused.
    let mut s = McState {
        focused_panel: McPanel::Activity,
        ..Default::default()
    };
    // Default window is Month.
    assert_eq!(s.analytics_window, TimeWindow::Month);

    // Each target differs from the current window in this sequence, so every
    // press is a genuine change.
    let cases = [
        (KeyCode::Char('d'), TimeWindow::Day),
        (KeyCode::Char('w'), TimeWindow::Week),
        (KeyCode::Char('A'), TimeWindow::All),
        (KeyCode::Char('m'), TimeWindow::Month),
    ];
    for (code, want) in cases {
        let out = decide(&mut s, 5, key(code));
        assert_eq!(out, KeyOutcome::AnalyticsWindowChanged, "key {code:?}");
        assert_eq!(s.analytics_window, want, "key {code:?}");
    }
}

#[test]
fn same_window_key_is_consumed_not_changed() {
    let mut s = McState {
        analytics_window: TimeWindow::Day,
        ..Default::default()
    };
    let out = decide(&mut s, 5, key(KeyCode::Char('d')));
    assert_eq!(out, KeyOutcome::Consumed);
    assert_eq!(s.analytics_window, TimeWindow::Day);
}

#[test]
fn filter_keys_are_global_across_panels() {
    // Inbox focused: 'w' still switches the window (the filter is global).
    let mut s = McState {
        focused_panel: McPanel::Inbox,
        ..Default::default()
    };
    let out = decide(&mut s, 5, key(KeyCode::Char('w')));
    assert_eq!(out, KeyOutcome::AnalyticsWindowChanged);
    assert_eq!(s.analytics_window, TimeWindow::Week);
}

#[test]
fn capital_a_is_all_but_lowercase_a_applies_in_inbox() {
    let mut s = McState {
        focused_panel: McPanel::Inbox,
        ..Default::default()
    };
    // Lowercase 'a' applies the selected inbox proposal and leaves the window.
    assert_eq!(
        decide(&mut s, 3, key(KeyCode::Char('a'))),
        KeyOutcome::ApplySelected
    );
    assert_eq!(
        s.analytics_window,
        TimeWindow::Month,
        "apply must not touch the window"
    );
    // Capital 'A' switches to the All window instead.
    assert_eq!(
        decide(&mut s, 3, key(KeyCode::Char('A'))),
        KeyOutcome::AnalyticsWindowChanged
    );
    assert_eq!(s.analytics_window, TimeWindow::All);
}

#[test]
fn scroll_keys_move_offset_in_analytics() {
    let mut s = McState {
        focused_panel: McPanel::Analytics,
        ..Default::default()
    };
    assert_eq!(s.scroll_offset, 0);
    assert_eq!(
        decide(&mut s, 5, key(KeyCode::Char('j'))),
        KeyOutcome::Consumed
    );
    assert_eq!(s.scroll_offset, 1, "j should scroll down");
    assert_eq!(
        decide(&mut s, 5, key(KeyCode::Char('k'))),
        KeyOutcome::Consumed
    );
    assert_eq!(s.scroll_offset, 0, "k should scroll back up");
    // k at zero saturates, never underflows.
    assert_eq!(
        decide(&mut s, 5, key(KeyCode::Char('k'))),
        KeyOutcome::Consumed
    );
    assert_eq!(s.scroll_offset, 0);
}

#[test]
fn home_resets_scroll_in_analytics() {
    let mut s = McState {
        focused_panel: McPanel::Analytics,
        scroll_offset: 10,
        ..Default::default()
    };
    decide(&mut s, 5, key(KeyCode::Home));
    assert_eq!(s.scroll_offset, 0, "Home should reset scroll");
}
