//! TUI Rendering
//!
//! Main rendering logic for the terminal interface.

mod chat;
mod dialogs;
mod help;
mod input;
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
    widgets::{Block, Borders, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use chat::render_chat;
use dialogs::{
    render_directory_picker, render_file_picker, render_model_selector, render_restart_dialog,
    render_usage_dialog,
};
use help::{render_help, render_settings};
use input::{render_input, render_slash_autocomplete, render_status_bar};
use sessions::render_sessions;
use utils::char_boundary_at_width_from_end;

/// Render the entire UI
pub fn render(f: &mut Frame, app: &mut App) {
    // Show splash screen if in splash mode - read directly from config
    if app.mode == AppMode::Splash {
        let config = crate::config::Config::load().unwrap_or_default();
        let (provider, model) = crate::config::resolve_provider_from_config(&config);
        splash::render_splash(f, f.area(), provider, model);
        return;
    }

    // Show onboarding wizard if in onboarding mode
    if app.mode == AppMode::Onboarding {
        if let Some(ref wizard) = app.onboarding {
            onboarding_render::render_onboarding(f, wizard);
        }
        return;
    }

    // Dynamic input height: 3 lines base (1 content + 2 border), grows with content
    let input_line_count = if app.input_buffer.is_empty() {
        1
    } else {
        let terminal_width = f.area().width.saturating_sub(4) as usize; // borders + padding
        app.input_buffer
            .lines()
            .map(|line| {
                if line.is_empty() {
                    1
                } else {
                    // Account for "  " padding prefix using display width
                    (UnicodeWidthStr::width(line) + 2).div_ceil(terminal_width.max(1))
                }
            })
            .sum::<usize>()
            .max(1)
    };
    let input_height = (input_line_count as u16 + 2).min(10); // +2 for borders, cap at 10

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),            // Header (1 content line + borders)
            Constraint::Min(10),              // Main content
            Constraint::Length(input_height), // Input (dynamic)
            Constraint::Length(1),            // Status bar
        ])
        .split(f.area());

    // Render components based on mode
    render_header(f, app, chunks[0]);

    // Merge main content + input + status bar for modes that don't need the input box
    let full_content_area = Rect {
        x: chunks[1].x,
        y: chunks[1].y,
        width: chunks[1].width,
        height: chunks[1].height + chunks[2].height + chunks[3].height,
    };

    match app.mode {
        AppMode::Splash => {
            // Already handled above
        }
        AppMode::Chat => {
            render_chat(f, app, chunks[1]);
            render_input(f, app, chunks[2]);
            render_status_bar(f, app, chunks[3]);
            // Render slash autocomplete dropdown above the input area
            if app.slash_suggestions_active {
                render_slash_autocomplete(f, app, chunks[2]);
            }
        }
        AppMode::Sessions => {
            render_sessions(f, app, full_content_area);
        }
        AppMode::Help => {
            render_help(f, app, full_content_area);
        }
        AppMode::Settings => {
            render_settings(f, app, full_content_area);
        }
        AppMode::FilePicker => {
            render_file_picker(f, app, full_content_area);
        }
        AppMode::DirectoryPicker => {
            render_directory_picker(f, app, full_content_area);
        }
        AppMode::ModelSelector => {
            render_chat(f, app, chunks[1]);
            render_input(f, app, chunks[2]);
            render_status_bar(f, app, chunks[3]);
            render_model_selector(f, app, f.area());
        }
        AppMode::UsageDialog => {
            render_chat(f, app, chunks[1]);
            render_input(f, app, chunks[2]);
            render_status_bar(f, app, chunks[3]);
            render_usage_dialog(f, app, f.area());
        }
        AppMode::RestartPending => {
            render_chat(f, app, chunks[1]);
            render_input(f, app, chunks[2]);
            render_status_bar(f, app, chunks[3]);
            render_restart_dialog(f, app, f.area());
        }
        AppMode::Onboarding => {
            // Handled by early return above
        }
    }
}

/// Render the header with working directory
fn render_header(f: &mut Frame, app: &App, area: Rect) {
    // Format working directory - show relative or full path
    let working_dir = app.working_directory.to_string_lossy().to_string();
    let display_dir = if working_dir.width() > 60 {
        // Take the last ~57 display-width chars, ensuring we split at a char boundary
        let suffix_start = char_boundary_at_width_from_end(&working_dir, 57);
        format!("...{}", &working_dir[suffix_start..])
    } else {
        working_dir
    };

    let header_line = Line::from(vec![
        Span::styled(" 📁 ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            display_dir,
            Style::default()
                .fg(Color::Rgb(90, 110, 150))
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    let header = Paragraph::new(vec![header_line]).block(
        Block::default()
            .borders(Borders::TOP | Borders::BOTTOM)
            .title(Span::styled(
                " 🦀 OpenCrabs AI Orchestration Agent ",
                Style::default()
                    .fg(Color::Rgb(120, 120, 120))
                    .add_modifier(Modifier::BOLD),
            ))
            .border_style(Style::default().fg(Color::Rgb(120, 120, 120))),
    );

    f.render_widget(header, area);
}
