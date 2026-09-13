//! Mission Control app-side state.
//!
//! Keeps every MC-specific field in one struct so `AppState` only holds
//! a single `pub mc: McState` field. Adding a new MC behaviour means a
//! new field on `McState`, not on `AppState`.

use crate::brain::mission_control::TimeWindow;

/// Which MC panel currently has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum McPanel {
    #[default]
    Inbox,
    Analytics,
    Activity,
    Schedule,
}

/// All Mission-Control-specific runtime state.
#[derive(Debug, Clone)]
pub struct McState {
    /// Which panel keyboard input affects.
    pub focused_panel: McPanel,
    /// Selected item index within the focused panel.
    pub selected_index: usize,
    /// Vertical scroll offset (rows scrolled past the top of the focused
    /// panel's content). Recomputed by the renderer to keep the
    /// selection visible.
    pub scroll_offset: u16,
    /// Whether the detail popup overlay is open.
    pub detail_open: bool,
    /// Cached activity feed entries — populated by `actions::refresh`,
    /// read by `activity_panel::draw`. Pre-fetched so each render frame
    /// is just a `&[McActivity]` borrow rather than a fresh disk read.
    pub activity: Vec<crate::brain::mission_control::McActivity>,
    /// Cached schedule rows — populated by `actions::refresh`, read by
    /// `schedule_panel::draw`. Same per-frame cost rationale as
    /// `activity`; sourced from the cron-jobs DB asynchronously so it
    /// can't run from inside the synchronous render path.
    pub schedule: Vec<crate::brain::mission_control::McScheduleItem>,
    /// Cached analytics snapshot — populated by `actions::refresh`, read
    /// by `analytics_panel::draw`. Brain sizes + tool/RSI stats from the
    /// same DB; pre-fetched so the render path stays synchronous.
    pub analytics: crate::brain::mission_control::McAnalytics,
    /// Active D/W/M/All filter for the analytics panel (#900). Switched
    /// with 1/2/3/4 while Analytics is focused; `actions::refresh` re-fetches
    /// the snapshot through this window so every panel respects it. Defaults
    /// to Month so first-open matches the prior 30d flakiest behavior.
    pub analytics_window: TimeWindow,
    /// Open log viewer, or `None` when MC is showing its panels (#1528).
    /// Held here rather than on `AppState` so closing it restores the panel
    /// focus that was never disturbed.
    pub log_viewer: Option<super::log_viewer::LogViewerState>,
}

impl Default for McState {
    fn default() -> Self {
        Self {
            focused_panel: McPanel::default(),
            selected_index: 0,
            scroll_offset: 0,
            detail_open: false,
            activity: Vec::new(),
            schedule: Vec::new(),
            analytics: crate::brain::mission_control::McAnalytics::default(),
            analytics_window: TimeWindow::Month,
            log_viewer: None,
        }
    }
}

impl McState {
    /// Reset focus + selection when re-entering Mission Control. Doesn't
    /// touch `detail_open` — that's owned by the popup open/close path.
    pub fn reset_focus(&mut self) {
        self.focused_panel = McPanel::default();
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    /// Cycle focus through panels in left → top-right → bottom-right
    /// order. Selection resets to the top of the new panel because
    /// indices aren't comparable across panels.
    pub fn focus_next(&mut self) {
        self.focused_panel = match self.focused_panel {
            McPanel::Inbox => McPanel::Analytics,
            McPanel::Analytics => McPanel::Activity,
            McPanel::Activity => McPanel::Schedule,
            McPanel::Schedule => McPanel::Inbox,
        };
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    /// Cycle focus backwards.
    pub fn focus_prev(&mut self) {
        self.focused_panel = match self.focused_panel {
            McPanel::Inbox => McPanel::Schedule,
            McPanel::Analytics => McPanel::Inbox,
            McPanel::Activity => McPanel::Analytics,
            McPanel::Schedule => McPanel::Activity,
        };
        self.selected_index = 0;
        self.scroll_offset = 0;
    }
}
