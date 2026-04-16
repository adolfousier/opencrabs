//! Dialog rendering
//!
//! File picker, directory picker, model selector, usage dialog, restart dialog, and update prompt.

use super::super::app::App;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

/// Render the file picker
pub(super) fn render_file_picker(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    // Header: directory path
    lines.push(Line::from(vec![
        Span::styled(
            "📁 File Picker",
            Style::default()
                .fg(Color::Rgb(120, 120, 120))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  │  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            app.file_picker_current_dir.to_string_lossy().to_string(),
            Style::default().fg(Color::Rgb(215, 100, 20)),
        ),
    ]));

    // Search bar
    if app.file_picker_search.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "  Type to filter...",
            Style::default().fg(Color::DarkGray),
        )]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("  🔍 ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                &app.file_picker_search,
                Style::default().fg(Color::Rgb(215, 100, 20)),
            ),
        ]));
    }
    lines.push(Line::from(""));

    let filtered = &app.file_picker_filtered;

    // Calculate visible range
    let visible_items = (area.height as usize).saturating_sub(7);
    let start = app.file_picker_scroll_offset;
    let end = (start + visible_items).min(filtered.len());

    // Render filtered file list
    for (display_idx, &file_idx) in filtered.iter().enumerate().skip(start).take(end - start) {
        let path = &app.file_picker_files[file_idx];
        let is_selected = display_idx == app.file_picker_selected;
        let is_dir = path.is_dir();

        let icon = if path.ends_with("..") {
            "📂 .."
        } else if is_dir {
            "📂"
        } else {
            "📄"
        };

        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");

        let style = if is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(120, 120, 120))
                .add_modifier(Modifier::BOLD)
        } else if is_dir {
            Style::default().fg(Color::Rgb(120, 120, 120))
        } else {
            Style::default().fg(Color::Reset)
        };

        let prefix = if is_selected { "▶ " } else { "  " };

        lines.push(Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(format!("{} {}", icon, filename), style),
        ]));
    }

    // Scroll indicator
    if filtered.len() > visible_items {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            format!("Showing {}-{} of {} files", start + 1, end, filtered.len()),
            Style::default().fg(Color::DarkGray),
        )]));
    }

    // Help text
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            "[↑↓]",
            Style::default()
                .fg(Color::Rgb(120, 120, 120))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Navigate  ", Style::default().fg(Color::Reset)),
        Span::styled(
            "[Enter]",
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Select  ", Style::default().fg(Color::Reset)),
        Span::styled(
            "[Esc]",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Cancel", Style::default().fg(Color::Reset)),
    ]));

    let widget = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(120, 120, 120)))
                .title(Span::styled(
                    " Select a file ",
                    Style::default()
                        .fg(Color::Rgb(120, 120, 120))
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .wrap(Wrap { trim: false });

    f.render_widget(widget, area);
}

/// Render directory picker (reuses file picker state, dirs only)
pub(super) fn render_directory_picker(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    // Header
    lines.push(Line::from(vec![
        Span::styled(
            "📂 Directory Picker",
            Style::default()
                .fg(Color::Rgb(120, 120, 120))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  │  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            app.file_picker_current_dir.to_string_lossy().to_string(),
            Style::default().fg(Color::Rgb(215, 100, 20)),
        ),
    ]));
    lines.push(Line::from(""));

    let visible_items = (area.height as usize).saturating_sub(6);
    let start = app.file_picker_scroll_offset;
    let end = (start + visible_items).min(app.file_picker_files.len());

    for (idx, path) in app
        .file_picker_files
        .iter()
        .enumerate()
        .skip(start)
        .take(end - start)
    {
        let is_selected = idx == app.file_picker_selected;

        let icon = if path.ends_with("..") {
            "📂 .."
        } else {
            "📂"
        };

        let filename = if path.ends_with("..") {
            ".."
        } else {
            path.file_name().and_then(|n| n.to_str()).unwrap_or("?")
        };

        let style = if is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(120, 120, 120))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Rgb(120, 120, 120))
        };

        let prefix = if is_selected { "▶ " } else { "  " };

        let display = if path.ends_with("..") {
            icon.to_string()
        } else {
            format!("{} {}", icon, filename)
        };

        lines.push(Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(display, style),
        ]));
    }

    if app.file_picker_files.len() > visible_items {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            format!(
                "Showing {}-{} of {}",
                start + 1,
                end,
                app.file_picker_files.len()
            ),
            Style::default().fg(Color::DarkGray),
        )]));
    }

    // Help text
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            "[↑↓]",
            Style::default()
                .fg(Color::Rgb(120, 120, 120))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Navigate  ", Style::default().fg(Color::Reset)),
        Span::styled(
            "[Enter]",
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Open  ", Style::default().fg(Color::Reset)),
        Span::styled(
            "[Space/Tab]",
            Style::default()
                .fg(Color::Rgb(60, 190, 190))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Select here  ", Style::default().fg(Color::Reset)),
        Span::styled(
            "[Esc]",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Cancel", Style::default().fg(Color::Reset)),
    ]));

    let widget = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(120, 120, 120)))
                .title(Span::styled(
                    " Change working directory ",
                    Style::default()
                        .fg(Color::Rgb(120, 120, 120))
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .wrap(Wrap { trim: false });

    f.render_widget(widget, area);
}

/// Render the model selector dialog - matches onboarding ProviderAuth style
pub(super) fn render_model_selector(f: &mut Frame, app: &App, area: Rect) {
    use crate::tui::onboarding::PROVIDERS;
    use crate::tui::provider_selector::{CUSTOM_INSTANCES_START, CUSTOM_PROVIDER_IDX};

    const BRAND_BLUE: Color = Color::Rgb(120, 120, 120);
    const BRAND_GOLD: Color = Color::Rgb(215, 100, 20);

    let focused_field = app.ps.focused_field; // 0=provider, 1=api_key, 2=model
    let provider_idx = app.ps.selected_provider;
    let clamped_idx = provider_idx.min(PROVIDERS.len() - 1);

    tracing::trace!(
        "[render_model_selector] provider_idx={}, clamped={}, PROVIDERS.len={}, custom_names={:?}, focused_field={}",
        provider_idx,
        clamped_idx,
        PROVIDERS.len(),
        app.ps.custom_names,
        focused_field,
    );

    let selected_provider = &PROVIDERS[clamped_idx];

    // Get models from fetched list, filtered by search text
    let filter = app.ps.model_filter.to_lowercase();
    let display_models: Vec<&str> = app
        .ps
        .models
        .iter()
        .filter(|m| filter.is_empty() || m.to_lowercase().contains(&filter))
        .map(|s| s.as_ref())
        .collect();

    let model_count = display_models.len();
    let current_model = app
        .current_session
        .as_ref()
        .and_then(|s| s.model.clone())
        .unwrap_or_else(|| app.provider_model());

    let custom_extra = app.ps.custom_names.len() as u16;
    let is_custom_selected = provider_idx >= CUSTOM_PROVIDER_IDX;
    // static providers + custom_extra + "+ New Custom" + API key line + filter + models + footer + padding
    let provider_lines = CUSTOM_PROVIDER_IDX as u16 + custom_extra + 1; // static + customs + new custom
    // Custom providers show text fields instead of model list:
    // Base URL(2) + API Key(2) + Model text(1) + Name(2) + Context Window(1) + spacing(2) + help(2) = 12
    let form_lines: u16 = if is_custom_selected {
        12
    } else if provider_idx == 10 {
        // Qwen: rotation toggle(2) + auth status(3) + model section(4) + footer(2) + padding(3)
        14 + model_count as u16
    } else {
        4 + model_count as u16 + 4 // key/filter chrome + model list + footer
    };
    let content_lines = provider_lines + form_lines;
    let max_height = (area.height * 3 / 4).max(20); // cap at 75% of terminal
    let dialog_height = content_lines
        .min(max_height)
        .min(area.height.saturating_sub(4));
    let dialog_width = 64u16.min(area.width * 9 / 10).max(40u16.min(area.width));

    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(dialog_height),
            Constraint::Min(0),
        ])
        .split(area);
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(dialog_width),
            Constraint::Min(0),
        ])
        .split(v_chunks[1]);
    let dialog_area = h_chunks[1];

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    // Provider list — static providers sorted alphabetically, then custom names, then "+ New Custom" last.
    let display_order = app.ps.provider_display_order();
    for &idx in &display_order {
        let selected = idx == provider_idx;
        let focused = focused_field == 0;
        let configured = app.ps.provider_has_credentials(idx);

        let prefix = if selected && focused { " > " } else { "   " };
        let marker = if selected { "[*]" } else { "[ ]" };

        let label = if idx == CUSTOM_PROVIDER_IDX {
            "+ New Custom Provider".to_string()
        } else if idx < PROVIDERS.len() {
            PROVIDERS[idx].name.to_string()
        } else {
            let custom_idx = idx - CUSTOM_INSTANCES_START;
            app.ps
                .custom_names
                .get(custom_idx)
                .cloned()
                .unwrap_or_else(|| "custom".to_string())
        };

        // Green for configured providers, white/bold for selected, gray for rest
        let label_color = if selected {
            Color::Reset
        } else if configured {
            Color::Green
        } else {
            Color::DarkGray
        };

        let mut spans = vec![
            Span::styled(prefix, Style::default().fg(BRAND_GOLD)),
            Span::styled(
                marker,
                Style::default().fg(if selected {
                    BRAND_GOLD
                } else if configured {
                    Color::Green
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(
                format!(" {}", label),
                Style::default().fg(label_color).add_modifier(if selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
            ),
        ];
        if configured && !selected {
            spans.push(Span::styled(" ✓", Style::default().fg(Color::Green)));
        }
        lines.push(Line::from(spans));
    }

    lines.push(Line::from(""));

    let is_custom = provider_idx >= CUSTOM_PROVIDER_IDX;

    // For Custom provider: show Base URL field first (field 1), then API Key (field 2)
    // For others: show API Key only (field 1)
    if is_custom {
        // Base URL field (field 1 for Custom)
        let base_focused = focused_field == 1;
        let base_display = if app.ps.base_url.is_empty() {
            "http://localhost:1234/v1".to_string()
        } else {
            app.ps.base_url.clone()
        };
        let cursor = if base_focused { "█" } else { "" };
        lines.push(Line::from(vec![
            Span::styled(
                "  Base URL: ",
                Style::default().fg(if base_focused {
                    BRAND_BLUE
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(
                format!("{}{}", base_display, cursor),
                Style::default().fg(if base_focused {
                    Color::Reset
                } else {
                    Color::Cyan
                }),
            ),
        ]));
        lines.push(Line::from(""));
    }

    // z.ai GLM endpoint type toggle (before API key)
    if provider_idx == 6 {
        let et_focused = focused_field == 1; // field 1 for zhipu = endpoint type
        let api_marker = if app.ps.zhipu_endpoint_type == 0 {
            "[*]"
        } else {
            "[ ]"
        };
        let coding_marker = if app.ps.zhipu_endpoint_type == 1 {
            "[*]"
        } else {
            "[ ]"
        };
        lines.push(Line::from(Span::styled(
            "  Endpoint Type:",
            Style::default().fg(if et_focused {
                BRAND_BLUE
            } else {
                Color::DarkGray
            }),
        )));
        lines.push(Line::from(vec![
            Span::styled(
                format!("    {} General API  ", api_marker),
                Style::default().fg(if et_focused && app.ps.zhipu_endpoint_type == 0 {
                    Color::Reset
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(
                format!("{} Coding API", coding_marker),
                Style::default().fg(if et_focused && app.ps.zhipu_endpoint_type == 1 {
                    Color::Reset
                } else {
                    Color::DarkGray
                }),
            ),
        ]));
        lines.push(Line::from(""));
    }

    // API Key field (field 1 for non-Custom, field 2 for Custom; field 2 for zhipu since field 1 = endpoint type)
    // CLI providers have no API key — skip entirely.
    let is_cli_provider = app.ps.is_cli();
    if !is_cli_provider {
        let is_zhipu = provider_idx == 6;
        let key_focused = (focused_field == 1 && !is_custom && !is_zhipu)
            || (focused_field == 2 && (is_custom || is_zhipu));
        let key_label = selected_provider.key_label;

        let has_existing_key = app.ps.has_existing_key;
        let has_user_key = !app.ps.api_key_input.is_empty();

        let (masked_key, key_hint) = if has_user_key {
            // User typed a new key - show asterisks for what they typed
            (
                "*".repeat(app.ps.api_key_input.len().min(30)),
                String::new(),
            )
        } else if has_existing_key {
            // Key exists in config - show placeholder indicating it's configured
            ("● configured".to_string(), String::new())
        } else {
            // Empty - show input hint
            (
                format!("enter your {} (optional)", key_label.to_lowercase()),
                String::new(),
            )
        };
        let cursor = if key_focused { "█" } else { "" };

        lines.push(Line::from(vec![
            Span::styled(
                format!("  {}: ", key_label),
                Style::default().fg(if key_focused {
                    BRAND_BLUE
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(
                format!("{}{}", masked_key, cursor),
                Style::default().fg(if key_focused {
                    Color::Reset
                } else {
                    Color::Cyan
                }),
            ),
        ]));

        if !key_hint.is_empty() && key_focused {
            lines.push(Line::from(Span::styled(
                format!("  {}", key_hint.trim()),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )));
        }

        lines.push(Line::from(""));
    } else if is_cli_provider {
        // CLI provider: show "no API key needed" hint
        let cli_name = match app.ps.provider_id() {
            "opencode-cli" => "opencode",
            "qwen-code-cli" => "qwen",
            _ => "claude",
        };
        lines.push(Line::from(Span::styled(
            format!("  No API key needed — uses local {} CLI", cli_name),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
        lines.push(Line::from(""));
    }

    // Model selection (field 2 for non-Custom, field 3 for Custom/zhipu)
    let is_zhipu_model = provider_idx == 6;
    let model_focused = (focused_field == 2 && !is_custom && !is_zhipu_model)
        || (focused_field == 3 && (is_custom || is_zhipu_model));
    const MAX_VISIBLE_MODELS: usize = 8;

    if is_custom {
        // Custom provider: free-text model name input (no filter/search)
        let model_cursor = if model_focused { "█" } else { "" };
        let model_display = if app.ps.custom_model.is_empty() {
            format!("enter model name (e.g. gpt-5-nano){}", model_cursor)
        } else {
            format!("{}{}", app.ps.custom_model, model_cursor)
        };
        lines.push(Line::from(vec![
            Span::styled(
                "  Model: ",
                Style::default().fg(if model_focused {
                    BRAND_BLUE
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(
                model_display,
                Style::default().fg(if model_focused {
                    Color::Reset
                } else if app.ps.custom_model.is_empty() {
                    Color::DarkGray
                } else {
                    Color::Cyan
                }),
            ),
        ]));
    } else {
        // Non-custom: filter/search model list
        if model_focused {
            let filter_cursor = if model_focused { "█" } else { "" };
            let filter_display = if app.ps.model_filter.is_empty() {
                format!("  / filter{}", filter_cursor)
            } else {
                format!("  / {}{}", app.ps.model_filter, filter_cursor)
            };
            lines.push(Line::from(Span::styled(
                filter_display,
                Style::default().fg(if model_focused {
                    BRAND_BLUE
                } else {
                    Color::DarkGray
                }),
            )));
        }

        let total = display_models.len();
        let max_sel = if total > 0 { total - 1 } else { 0 };
        let safe_selected = app.ps.selected_model.min(max_sel);
        let (start, end) = if total <= MAX_VISIBLE_MODELS {
            (0, total)
        } else {
            let half = MAX_VISIBLE_MODELS / 2;
            let s = safe_selected
                .saturating_sub(half)
                .min(total - MAX_VISIBLE_MODELS);
            (s, s + MAX_VISIBLE_MODELS)
        };

        if start > 0 {
            lines.push(Line::from(Span::styled(
                format!("  ↑ {} more", start),
                Style::default().fg(Color::DarkGray),
            )));
        }

        for (offset, model) in display_models[start..end].iter().enumerate() {
            let i = start + offset;
            let selected = i == safe_selected;
            let active = *model == current_model;

            let prefix = if selected && model_focused {
                " > "
            } else {
                "   "
            };

            let style = if selected && model_focused {
                Style::default()
                    .fg(Color::Black)
                    .bg(BRAND_BLUE)
                    .add_modifier(Modifier::BOLD)
            } else if active {
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Reset)
            };

            let suffix = if active { " (active)" } else { "" };
            let label = crate::tui::provider_selector::model_display_label(model);

            lines.push(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(label.to_string(), style),
                Span::styled(suffix, Style::default().fg(Color::DarkGray)),
            ]));
        }

        if end < total {
            lines.push(Line::from(Span::styled(
                format!("  ↓ {} more", total - end),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    // Custom provider: name identifier field (field 4 — last before save)
    if is_custom {
        let name_focused = focused_field == 4;
        let name_cursor = if name_focused { "█" } else { "" };
        let name_display = if app.ps.custom_name.is_empty() {
            format!("enter identifier (e.g. nvidia, kimi){}", name_cursor)
        } else {
            format!("{}{}", app.ps.custom_name, name_cursor)
        };
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                "  Name: ",
                Style::default().fg(if name_focused {
                    BRAND_BLUE
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(
                name_display,
                Style::default().fg(if name_focused {
                    Color::Reset
                } else if app.ps.custom_name.is_empty() {
                    Color::DarkGray
                } else {
                    Color::Cyan
                }),
            ),
        ]));

        // Context Window field (field 5)
        let cw_focused = focused_field == 5;
        let cw_cursor = if cw_focused { "█" } else { "" };
        let cw_display = if app.ps.context_window.is_empty() {
            format!("e.g. 128000 (optional){}", cw_cursor)
        } else {
            format!("{}{}", app.ps.context_window, cw_cursor)
        };
        lines.push(Line::from(vec![
            Span::styled(
                "  Context Window: ",
                Style::default().fg(if cw_focused {
                    BRAND_BLUE
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(
                cw_display,
                Style::default().fg(if cw_focused {
                    Color::Reset
                } else if app.ps.context_window.is_empty() {
                    Color::DarkGray
                } else {
                    Color::Cyan
                }),
            ),
        ]));
    }

    lines.push(Line::from(""));

    // Help text - show different instructions based on focused field
    let help_text = if is_custom {
        match focused_field {
            0 => vec![
                ("[↑/↓]", "Select"),
                ("[Enter]", "Next"),
                ("[Tab]", "Skip to Model"),
            ],
            1 => vec![("[Type]", "Base URL"), ("[Enter]", "Next")],
            2 => vec![("[Type]", "API Key"), ("[Enter]", "Next")],
            3 => vec![("[Type]", "Model name"), ("[Enter]", "Next")],
            4 => vec![("[Type]", "Provider name"), ("[Enter]", "Next")],
            5 => vec![("[Type]", "Context window (tokens)"), ("[Enter]", "Save")],
            _ => vec![],
        }
    } else {
        match focused_field {
            0 => vec![
                ("[↑/↓]", "Select"),
                ("[Enter]", "Next"),
                ("[Tab]", "Skip to Model"),
            ],
            1 => vec![("[Type]", "API Key"), ("[Enter]", "Fetch Models")],
            2 => vec![
                ("[Type]", "Filter"),
                ("[↑/↓]", "Select"),
                ("[Enter]", "Confirm"),
            ],
            _ => vec![],
        }
    };

    let mut help_spans: Vec<Span> = Vec::new();
    help_spans.push(Span::raw("   "));
    for (key, action) in help_text {
        help_spans.push(Span::styled(
            key,
            Style::default()
                .fg(Color::Rgb(215, 100, 20))
                .add_modifier(Modifier::BOLD),
        ));
        help_spans.push(Span::styled(
            format!("{}  ", action),
            Style::default().fg(Color::Reset),
        ));
    }
    lines.push(Line::from(help_spans));

    f.render_widget(Clear, dialog_area);
    let dialog = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(BRAND_BLUE))
            .title(Span::styled(
                " Select Provider & Model ",
                Style::default().fg(BRAND_BLUE).add_modifier(Modifier::BOLD),
            )),
    );
    f.render_widget(dialog, dialog_area);
}

/// Render restart confirmation dialog
pub(super) fn render_restart_dialog(f: &mut Frame, app: &App, area: Rect) {
    let status = app.rebuild_status.as_deref().unwrap_or("Build successful");

    let dialog_height = 8u16;
    let dialog_width = 50u16.min(area.width.saturating_sub(4));

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Length(dialog_height),
            Constraint::Percentage(40),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min((area.width.saturating_sub(dialog_width)) / 2),
            Constraint::Length(dialog_width),
            Constraint::Min(0),
        ])
        .split(vertical[1]);

    let dialog_area = horizontal[1];
    f.render_widget(Clear, dialog_area);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", status),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  Restart with new binary?"),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  [Enter] ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Restart  "),
            Span::styled(
                "[Esc] ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw("Cancel"),
        ]),
    ];

    let dialog = Paragraph::new(lines).block(
        Block::default()
            .title(" Rebuild Complete ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(dialog, dialog_area);
}

/// Render update prompt dialog
pub(super) fn render_update_dialog(f: &mut Frame, app: &App, area: Rect) {
    let version = app.update_available_version.as_deref().unwrap_or("unknown");

    let dialog_height = 8u16;
    let dialog_width = 55u16.min(area.width.saturating_sub(4));

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Length(dialog_height),
            Constraint::Percentage(40),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min((area.width.saturating_sub(dialog_width)) / 2),
            Constraint::Length(dialog_width),
            Constraint::Min(0),
        ])
        .split(vertical[1]);

    let dialog_area = horizontal[1];
    f.render_widget(Clear, dialog_area);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  v{} -> v{}", crate::VERSION, version),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  Update now?"),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  [Enter] ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Update  "),
            Span::styled(
                "[Esc] ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw("Skip"),
        ]),
    ];

    let dialog = Paragraph::new(lines).block(
        Block::default()
            .title(" Update Available ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(dialog, dialog_area);
}
