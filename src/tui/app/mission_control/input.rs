//! Keyboard handling for `AppMode::MissionControl`.
//!
//! Three layers:
//!
//!  1. Detail popup open → Esc closes the popup. j/k still scroll the
//!     selection underneath so the popup updates as the user moves.
//!  2. Panel focused, no popup → Tab/Shift-Tab cycle panels;
//!     j/k or ↑/↓ move selection within the focused panel; Enter
//!     opens the detail popup; Esc closes MC entirely.
//!  3. Apply / reject (`a` / `r`) land in C12 alongside the
//!     `rsi_proposals` action plumbing.
//!
//! The decision logic is split into a pure `decide` function that takes
//! a `&mut McState` plus the current panel item count, and returns a
//! `KeyOutcome`. The `handle_key` wrapper at the top routes that
//! outcome back into `App`-level effects (mode switch on Close). This
//! keeps the keystroke logic unit-testable without spinning up a full
//! `App`.

use super::state::{McPanel, McState};
use crate::brain::mission_control::TimeWindow;
use crate::tui::app::App;
use crate::tui::events::AppMode;
use crossterm::event::{KeyCode, KeyEvent};

/// Upper bound on the analytics panel's body scroll so holding `j` past the
/// bottom can't wind `scroll_offset` far beyond the content (#900). The
/// renderer clamps visually too; this just keeps the stored value sane.
const MAX_ANALYTICS_SCROLL: u16 = 150;

/// Effect of a keystroke that the wrapper has to apply at the App level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    /// Key consumed; no further App-level action required.
    Consumed,
    /// User wants to leave MC — caller should switch back to Chat mode.
    Close,
    /// Key wasn't recognised; caller may fall through to the chat-mode
    /// default handlers.
    NotConsumed,
    /// Inbox panel is focused and the user pressed `a` — caller should
    /// apply the currently selected proposal via `actions::apply_selected`.
    ApplySelected,
    /// Inbox panel is focused and the user pressed `r` — caller should
    /// reject the currently selected proposal.
    RejectSelected,
    /// The user switched the D/W/M/All window (#900). Global to every
    /// panel — caller should re-fetch the snapshot via
    /// `actions::refresh_analytics`.
    AnalyticsWindowChanged,
}

/// Top-level handler called from the App's keystroke dispatcher.
/// Mission Control is a full-screen mode (like `Sessions` and `Help`) —
/// the App's match-arm doesn't fall through to chat handlers, so there
/// is no return value to thread back. Unrecognised keys are simply
/// ignored.
pub async fn handle_key(app: &mut App, key: KeyEvent) {
    let count = panel_count(app);
    match decide(&mut app.mc, count, key) {
        KeyOutcome::Consumed | KeyOutcome::NotConsumed => {}
        KeyOutcome::Close => {
            app.mode = AppMode::Chat;
            app.mc.detail_open = false;
        }
        KeyOutcome::ApplySelected => super::actions::apply_selected(app).await,
        KeyOutcome::RejectSelected => super::actions::reject_selected(app).await,
        KeyOutcome::AnalyticsWindowChanged => super::actions::refresh_analytics(app).await,
    }
}

/// Pure decision function — mutates `state`, returns the App-level
/// effect. `panel_item_count` is the number of items in the currently
/// focused panel, used to clamp selection movement.
pub fn decide(state: &mut McState, panel_item_count: usize, key: KeyEvent) -> KeyOutcome {
    // The log viewer is full-screen, so it takes every key while open (#1528).
    // Ahead of the popup check because the two are never open together.
    if state.log_viewer.is_some() {
        return decide_in_log_viewer(state, key);
    }
    if state.detail_open {
        decide_with_popup(state, panel_item_count, key)
    } else {
        decide_without_popup(state, panel_item_count, key)
    }
}

/// Keys inside the log viewer (#1528).
///
/// Returns `Consumed` for everything, including keys with no binding: a
/// stray keystroke must not fall through to the panels underneath and move a
/// selection the user cannot see.
fn decide_in_log_viewer(state: &mut McState, key: KeyEvent) -> KeyOutcome {
    let Some(viewer) = state.log_viewer.as_mut() else {
        return KeyOutcome::NotConsumed;
    };

    // While the search box is open, printable keys are search text rather
    // than commands — otherwise typing "debug" would trip the `d` level
    // filter and the `e` one on the way past.
    if viewer.search_active {
        match key.code {
            KeyCode::Esc => viewer.clear_search(),
            KeyCode::Enter => viewer.search_active = false,
            KeyCode::Backspace => viewer.pop_search(),
            KeyCode::Char(c) if !c.is_control() => viewer.push_search(c),
            _ => {}
        }
        return KeyOutcome::Consumed;
    }

    // Esc clears a filter first and closes second, so a search that matched
    // nothing costs one key rather than the whole screen. Decided here and
    // acted on after the borrow ends, since closing means dropping the state
    // this borrow points into.
    let mut close = false;
    match key.code {
        KeyCode::Esc => {
            if viewer.search.is_empty() {
                close = true;
            } else {
                viewer.clear_search();
            }
        }
        KeyCode::Char('/') => viewer.search_active = true,
        KeyCode::Up | KeyCode::Char('k') => viewer.scroll_up(1),
        KeyCode::Down | KeyCode::Char('j') => viewer.scroll_down(1),
        KeyCode::PageUp => viewer.page_up(),
        KeyCode::PageDown => viewer.page_down(),
        KeyCode::Home | KeyCode::Char('g') => viewer.scroll = 0,
        KeyCode::End | KeyCode::Char('G') => viewer.scroll_to_end(),
        KeyCode::Char('[') => viewer.step_file(-1),
        KeyCode::Char(']') => viewer.step_file(1),
        KeyCode::Char(c) => {
            if let Some(level) = super::log_viewer::level_for_key(c) {
                viewer.set_level(level);
            }
        }
        _ => {}
    }
    if close {
        state.log_viewer = None;
    }
    KeyOutcome::Consumed
}

fn decide_with_popup(state: &mut McState, panel_item_count: usize, key: KeyEvent) -> KeyOutcome {
    match key.code {
        KeyCode::Esc => {
            state.detail_open = false;
            KeyOutcome::Consumed
        }
        // Allow scrolling the underlying selection so the popup updates
        // as the user moves through the list. The analytics popup is a
        // dashboard (no per-row selection), so j/k scroll its body instead.
        KeyCode::Up | KeyCode::Char('k') => {
            if state.focused_panel == McPanel::Analytics {
                state.scroll_offset = state.scroll_offset.saturating_sub(1);
            } else {
                move_selection(state, panel_item_count, -1);
            }
            KeyOutcome::Consumed
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if state.focused_panel == McPanel::Analytics {
                state.scroll_offset = state
                    .scroll_offset
                    .saturating_add(1)
                    .min(MAX_ANALYTICS_SCROLL);
            } else {
                move_selection(state, panel_item_count, 1);
            }
            KeyOutcome::Consumed
        }
        _ => KeyOutcome::NotConsumed,
    }
}

fn decide_without_popup(state: &mut McState, panel_item_count: usize, key: KeyEvent) -> KeyOutcome {
    match key.code {
        KeyCode::Esc => KeyOutcome::Close,
        // Uppercase, because lowercase `l` is the vim-right panel cycle
        // below and taking it would break panel navigation (#1528).
        KeyCode::Char('L') => {
            let mut viewer = super::log_viewer::LogViewerState::open();
            // Land on the newest entries: the thing you just reproduced is at
            // the end of the file, not the start of the window.
            viewer.scroll_to_end();
            state.log_viewer = Some(viewer);
            KeyOutcome::Consumed
        }
        KeyCode::Tab | KeyCode::Char('l') => {
            state.focus_next();
            KeyOutcome::Consumed
        }
        KeyCode::BackTab | KeyCode::Char('h') => {
            state.focus_prev();
            KeyOutcome::Consumed
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if state.focused_panel == McPanel::Analytics {
                state.scroll_offset = state.scroll_offset.saturating_sub(1);
            } else {
                move_selection(state, panel_item_count, -1);
            }
            KeyOutcome::Consumed
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if state.focused_panel == McPanel::Analytics {
                state.scroll_offset = state
                    .scroll_offset
                    .saturating_add(1)
                    .min(MAX_ANALYTICS_SCROLL);
            } else {
                move_selection(state, panel_item_count, 1);
            }
            KeyOutcome::Consumed
        }
        KeyCode::Home | KeyCode::Char('g') => {
            if state.focused_panel == McPanel::Analytics {
                state.scroll_offset = 0;
            } else {
                state.selected_index = 0;
            }
            KeyOutcome::Consumed
        }
        KeyCode::End | KeyCode::Char('G') => {
            state.selected_index = panel_item_count.saturating_sub(1);
            KeyOutcome::Consumed
        }
        KeyCode::Enter => {
            if panel_item_count > 0 {
                state.detail_open = true;
            }
            KeyOutcome::Consumed
        }
        // Apply / reject are scoped to the Inbox panel — that's where
        // the actionable items live. On other panels these keys are
        // swallowed (Consumed) rather than falling through, so they
        // can't accidentally trigger anything in the chat handlers.
        KeyCode::Char('a') => {
            if state.focused_panel == McPanel::Inbox && panel_item_count > 0 {
                KeyOutcome::ApplySelected
            } else {
                KeyOutcome::Consumed
            }
        }
        KeyCode::Char('r') => {
            if state.focused_panel == McPanel::Inbox && panel_item_count > 0 {
                KeyOutcome::RejectSelected
            } else {
                KeyOutcome::Consumed
            }
        }
        // D/W/M/A filter keys (#900): global to the whole Mission Control
        // view, so the analytics window switches from any panel, not just
        // Analytics. The active window is shown in the bottom commands bar.
        // The wrapper re-fetches the snapshot through the new window
        // (AnalyticsWindowChanged); switching resets the body scroll so the
        // re-windowed view starts at the top. Capital `A` is "All" and is
        // distinct from lowercase `a` (apply) matched above.
        c @ (KeyCode::Char('d') | KeyCode::Char('w') | KeyCode::Char('m') | KeyCode::Char('A')) => {
            let window = match c {
                KeyCode::Char('d') => TimeWindow::Day,
                KeyCode::Char('w') => TimeWindow::Week,
                KeyCode::Char('m') => TimeWindow::Month,
                _ => TimeWindow::All,
            };
            if state.analytics_window == window {
                KeyOutcome::Consumed
            } else {
                state.analytics_window = window;
                state.scroll_offset = 0;
                KeyOutcome::AnalyticsWindowChanged
            }
        }
        _ => KeyOutcome::NotConsumed,
    }
}

fn move_selection(state: &mut McState, count: usize, delta: i32) {
    if count == 0 {
        state.selected_index = 0;
        return;
    }
    let max_idx = count - 1;
    let cur = state.selected_index.min(max_idx) as i32;
    let next = (cur + delta).clamp(0, max_idx as i32) as usize;
    state.selected_index = next;
}

fn panel_count(app: &App) -> usize {
    match app.mc.focused_panel {
        // Inbox count is recomputed each draw from the proposals store
        // rather than cached in McState. Reading it here means a
        // fresh disk read on every Enter / Tab keystroke, which stays
        // in sync if the inbox file changes mid-session.
        McPanel::Inbox => crate::brain::mission_control::inbox_service::list().len(),
        McPanel::Activity => app.mc.activity.len(),
        McPanel::Schedule => app.mc.schedule.len(),
        // One "item" (the whole snapshot) so Enter opens the full-detail popup
        // like the other panels; there are no per-row selections.
        McPanel::Analytics => 1,
    }
}
