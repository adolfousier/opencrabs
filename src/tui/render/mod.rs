//! TUI Rendering
//!
//! Main rendering logic for the terminal interface.

mod chat;
mod dialogs;
mod help;
mod input;
mod panes;
mod plan_widget;
mod sessions;
mod tools;
mod utils;

// Re-export for sibling modules (e.g. onboarding_render)
pub(in crate::tui) use utils::char_boundary_at_width;

// Re-export for tests
#[cfg(test)]
pub(crate) use chat::reasoning_to_lines;
#[cfg(test)]
pub(crate) use tools::collapse_build_output;

use super::app::App;
use super::events::AppMode;
use super::onboarding_render;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use chat::render_chat;
use dialogs::{
    render_directory_picker, render_file_picker, render_model_selector, render_restart_dialog,
    render_update_dialog, render_usage_dialog,
};
use help::{render_help, render_settings};
use input::{render_emoji_picker, render_input, render_slash_autocomplete, render_status_bar};
use plan_widget::render_plan_checklist;
use sessions::render_sessions;

/// Render the entire UI
pub fn render(f: &mut Frame, app: &mut App) {
    if app.mode == AppMode::Onboarding {
        if let Some(ref wizard) = app.onboarding {
            onboarding_render::render_onboarding(f, wizard);
        }
        return;
    }

    // Dynamic input height: grows with content, capped at 10.
    // Early-exit once we hit the cap so huge pastes don't walk every line.
    let input_height = if app.input_buffer.is_empty() {
        3 // 1 content row + 2 borders
    } else {
        let terminal_width = (f.area().width.saturating_sub(4) as usize).max(1);
        const MAX_CONTENT_ROWS: usize = 8; // 10 cap - 2 borders
        let mut rows = 0usize;
        for line in app.input_buffer.lines() {
            rows += if line.is_empty() {
                1
            } else {
                (UnicodeWidthStr::width(line) + 2).div_ceil(terminal_width)
            };
            if rows >= MAX_CONTENT_ROWS {
                rows = MAX_CONTENT_ROWS;
                break;
            }
        }
        (rows.max(1) as u16 + 2).min(10)
    };

    // Show the plan checklist only while tasks are actively executing.
    // Any other status means the plan is not running (user moved on, cancelled, etc.).
    let plan_height = app
        .plan_document
        .as_ref()
        .filter(|p| p.status == crate::tui::plan::PlanStatus::InProgress)
        .map(|p| (p.tasks.len() + 2).min(8) as u16)
        .unwrap_or(0);

    // "OpenCrabs is thinking..." spinner is now rendered inline in chat.rs
    // (above the tool group) so it scrolls with content and stays above
    // the tool processing indicator.

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(10),              // [0] Chat messages
            Constraint::Length(plan_height),  // [1] Plan checklist (0 when no plan)
            Constraint::Length(input_height), // [2] Input (dynamic)
            Constraint::Length(1),            // [3] Status bar
        ])
        .split(f.area());

    // Full area for modes that replace the chat+input (Sessions, Help, etc.)
    // These modes do not show the plan checklist.
    let full_content_area = Rect {
        x: chunks[0].x,
        y: chunks[0].y,
        width: chunks[0].width,
        height: chunks[0].height + chunks[1].height + chunks[2].height + chunks[3].height,
    };

    match app.mode {
        AppMode::Onboarding => {
            // Handled by early return above
        }
        AppMode::Chat => {
            if app.pane_manager.is_split() {
                render_split_panes(f, app, chunks[0]);
            } else {
                render_chat(f, app, chunks[0]);
            }
            if plan_height > 0 {
                render_plan_checklist(f, app, chunks[1]);
            }
            render_input(f, app, chunks[2]);
            render_status_bar(f, app, chunks[3]);
            if app.slash_suggestions_active {
                render_slash_autocomplete(f, app, chunks[2]);
            } else if app.emoji_picker_active {
                render_emoji_picker(f, app, chunks[2]);
            }
        }
        AppMode::Sessions => {
            // Clear the full area first to prevent artifacts from split panes
            f.render_widget(Clear, full_content_area);
            let (title_area, content_area) = split_title_area(full_content_area);
            render_app_title(f, title_area);
            render_sessions(f, app, content_area);
        }
        AppMode::Help => {
            let (title_area, content_area) = split_title_area(full_content_area);
            render_app_title(f, title_area);
            render_help(f, app, content_area);
        }
        AppMode::Settings => {
            let (title_area, content_area) = split_title_area(full_content_area);
            render_app_title(f, title_area);
            render_settings(f, app, content_area);
        }
        AppMode::FilePicker => {
            render_file_picker(f, app, full_content_area);
        }
        AppMode::DirectoryPicker => {
            render_directory_picker(f, app, full_content_area);
        }
        AppMode::ModelSelector => {
            render_chat(f, app, chunks[0]);
            if plan_height > 0 {
                render_plan_checklist(f, app, chunks[1]);
            }
            render_input(f, app, chunks[2]);
            render_status_bar(f, app, chunks[3]);
            render_model_selector(f, app, f.area());
        }
        AppMode::UsageDialog => {
            render_chat(f, app, chunks[0]);
            if plan_height > 0 {
                render_plan_checklist(f, app, chunks[1]);
            }
            render_input(f, app, chunks[2]);
            render_status_bar(f, app, chunks[3]);
            render_usage_dialog(f, app, f.area());
        }
        AppMode::RestartPending => {
            render_chat(f, app, chunks[0]);
            if plan_height > 0 {
                render_plan_checklist(f, app, chunks[1]);
            }
            render_input(f, app, chunks[2]);
            render_status_bar(f, app, chunks[3]);
            render_restart_dialog(f, app, f.area());
        }
        AppMode::UpdatePrompt => {
            // Overlay the update dialog on top of the normal chat UI.
            if app.pane_manager.is_split() {
                render_split_panes(f, app, chunks[0]);
            } else {
                render_chat(f, app, chunks[0]);
            }
            if plan_height > 0 {
                render_plan_checklist(f, app, chunks[1]);
            }
            render_input(f, app, chunks[2]);
            render_status_bar(f, app, chunks[3]);
            render_update_dialog(f, app, f.area());
        }
    }
}

/// Render the chat area as split panes.
fn render_split_panes(f: &mut Frame, app: &mut App, area: Rect) {
    let tree = match &app.pane_manager.root {
        Some(t) => t.clone(),
        None => return render_chat(f, app, area),
    };

    let focused_id = app.pane_manager.focused;
    let pane_rects = tree.layout(area);

    for (pane_id, rect) in pane_rects {
        if rect.width < 3 || rect.height < 3 {
            continue; // too small to render
        }
        if pane_id == focused_id {
            let inner = panes::focused_pane_border(f, app, rect);
            render_chat(f, app, inner);
        } else {
            panes::render_inactive_pane(f, app, pane_id, rect);
        }
    }
}

/// Split 1 row off the top of an area for the app title bar.
fn split_title_area(area: Rect) -> (Rect, Rect) {
    let title_height = 1u16; // title only
    let used_title = title_height.min(area.height);
    let title_area = Rect {
        height: used_title,
        ..area
    };
    // Clamp content_area.y so it never lands past the buffer when area is
    // very small (e.g. height == 0 during a resize). Without the clamp,
    // `area.y + title_height` can walk one row past the valid buffer and
    // downstream renders panic on the first cell write.
    let content_area = Rect {
        y: area.y.saturating_add(used_title),
        height: area.height.saturating_sub(used_title),
        ..area
    };
    (title_area, content_area)
}

/// Render the app name header used on Sessions, Help, and Settings screens.
fn render_app_title(f: &mut Frame, area: Rect) {
    let para = Paragraph::new(vec![Line::from(Span::styled(
        " 🦀 OpenCrabs AI Orchestration Agent",
        Style::default()
            .fg(Color::Rgb(120, 120, 120))
            .add_modifier(Modifier::BOLD),
    ))]);
    f.render_widget(para, area);
}
