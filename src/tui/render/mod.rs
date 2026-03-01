//! TUI Rendering
//!
//! Main rendering logic for the terminal interface.

mod chat;
mod dialogs;
mod help;
mod input;
mod plan_widget;
mod sessions;
mod tools;
mod utils;

// Re-export for sibling modules (e.g. onboarding_render)
pub(in crate::tui) use utils::char_boundary_at_width;

use super::app::App;
use super::events::AppMode;
use super::onboarding_render;
use super::splash;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_width::UnicodeWidthStr;

use chat::render_chat;
use dialogs::{
    render_directory_picker, render_file_picker, render_model_selector, render_restart_dialog,
    render_usage_dialog,
};
use help::{render_help, render_settings};
use input::{render_input, render_slash_autocomplete, render_status_bar};
use plan_widget::render_plan_checklist;
use sessions::render_sessions;

/// Render the entire UI
pub fn render(f: &mut Frame, app: &mut App) {
    if app.mode == AppMode::Splash {
        let config = crate::config::Config::load().unwrap_or_default();
        let (provider, model) = crate::config::resolve_provider_from_config(&config);
        splash::render_splash(f, f.area(), provider, model);
        return;
    }

    if app.mode == AppMode::Onboarding {
        if let Some(ref wizard) = app.onboarding {
            onboarding_render::render_onboarding(f, wizard);
        }
        return;
    }

    // Dynamic input height: grows with content, capped at 10
    let input_line_count = if app.input_buffer.is_empty() {
        1
    } else {
        let terminal_width = f.area().width.saturating_sub(4) as usize;
        app.input_buffer
            .lines()
            .map(|line| {
                if line.is_empty() {
                    1
                } else {
                    (UnicodeWidthStr::width(line) + 2).div_ceil(terminal_width.max(1))
                }
            })
            .sum::<usize>()
            .max(1)
    };
    let input_height = (input_line_count as u16 + 2).min(10);

    // Show the plan checklist only while tasks are actively executing.
    // Any other status means the plan is not running (user moved on, cancelled, etc.).
    let plan_height = app
        .plan_document
        .as_ref()
        .filter(|p| p.status == crate::tui::plan::PlanStatus::InProgress)
        .map(|p| (p.tasks.len() + 2).min(8) as u16)
        .unwrap_or(0);

    // Sticky "OpenCrabs is thinking..." row: visible only during the brief
    // window between submitting a prompt and the first streaming token/tool.
    let thinking_height: u16 = if !app.has_pending_approval()
        && app.is_processing
        && app.streaming_response.is_none()
        && app.streaming_reasoning.is_none()
        && app.active_tool_group.is_none()
    {
        1
    } else {
        0
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(10),                 // [0] Chat messages
            Constraint::Length(plan_height),     // [1] Plan checklist (0 when no plan)
            Constraint::Length(thinking_height), // [2] Thinking indicator (0 or 1)
            Constraint::Length(input_height),    // [3] Input (dynamic)
            Constraint::Length(1),               // [4] Status bar
        ])
        .split(f.area());

    // Full area for modes that replace the chat+input (Sessions, Help, etc.)
    // These modes do not show the plan checklist.
    let full_content_area = Rect {
        x: chunks[0].x,
        y: chunks[0].y,
        width: chunks[0].width,
        height: chunks[0].height
            + chunks[1].height
            + chunks[2].height
            + chunks[3].height
            + chunks[4].height,
    };

    match app.mode {
        AppMode::Splash | AppMode::Onboarding => {
            // Handled by early returns above
        }
        AppMode::Chat => {
            render_chat(f, app, chunks[0]);
            if plan_height > 0 {
                render_plan_checklist(f, app, chunks[1]);
            }
            if thinking_height > 0 {
                render_thinking_indicator(f, app, chunks[2]);
            }
            render_input(f, app, chunks[3]);
            render_status_bar(f, app, chunks[4]);
            if app.slash_suggestions_active {
                render_slash_autocomplete(f, app, chunks[3]);
            }
        }
        AppMode::Sessions => {
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
            if thinking_height > 0 {
                render_thinking_indicator(f, app, chunks[2]);
            }
            render_input(f, app, chunks[3]);
            render_status_bar(f, app, chunks[4]);
            render_model_selector(f, app, f.area());
        }
        AppMode::UsageDialog => {
            render_chat(f, app, chunks[0]);
            if plan_height > 0 {
                render_plan_checklist(f, app, chunks[1]);
            }
            if thinking_height > 0 {
                render_thinking_indicator(f, app, chunks[2]);
            }
            render_input(f, app, chunks[3]);
            render_status_bar(f, app, chunks[4]);
            render_usage_dialog(f, app, f.area());
        }
        AppMode::RestartPending => {
            render_chat(f, app, chunks[0]);
            if plan_height > 0 {
                render_plan_checklist(f, app, chunks[1]);
            }
            if thinking_height > 0 {
                render_thinking_indicator(f, app, chunks[2]);
            }
            render_input(f, app, chunks[3]);
            render_status_bar(f, app, chunks[4]);
            render_restart_dialog(f, app, f.area());
        }
    }
}

/// Render the sticky "OpenCrabs is thinking..." spinner row.
/// This sits between the chat area and the input box so it is always visible
/// and never scrolls away with chat history.
fn render_thinking_indicator(f: &mut Frame, app: &App, area: Rect) {
    let spinner_frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let frame = spinner_frames[app.animation_frame % spinner_frames.len()];
    let line = Line::from(vec![
        Span::styled(
            format!("  {} ", frame),
            Style::default()
                .fg(Color::Rgb(120, 120, 120))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "OpenCrabs is thinking...",
            Style::default().fg(Color::Rgb(215, 100, 20)),
        ),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

/// Split 1 row off the top of an area for the app title bar.
fn split_title_area(area: Rect) -> (Rect, Rect) {
    let title_height = 1u16; // title only
    let title_area = Rect {
        height: title_height.min(area.height),
        ..area
    };
    let content_area = Rect {
        y: area.y + title_height,
        height: area.height.saturating_sub(title_height),
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
