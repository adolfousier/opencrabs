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

    // Header
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
    lines.push(Line::from(""));

    // Calculate visible range
    let visible_items = (area.height as usize).saturating_sub(6); // Leave space for header and help
    let start = app.file_picker_scroll_offset;
    let end = (start + visible_items).min(app.file_picker_files.len());

    // Render file list
    for (idx, path) in app
        .file_picker_files
        .iter()
        .enumerate()
        .skip(start)
        .take(end - start)
    {
        let is_selected = idx == app.file_picker_selected;
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

    // Add scroll indicator if needed
    if app.file_picker_files.len() > visible_items {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            format!(
                "Showing {}-{} of {} files",
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
    let dialog_width = 64u16.min(area.width.saturating_sub(4));

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

    // Qwen native (idx 10) uses OAuth device flow instead of an API key.
    // Render the same status block as `/onboard` so the user gets a
    // consistent experience across the two surfaces.
    if provider_idx == 10 {
        use crate::tui::onboarding::QwenDeviceFlowStatus;

        // Rotation toggle — only in auth area (focused_field >= 1), not provider list
        if app.ps.focused_field >= 1 {
            let rotation_marker = if app.ps.qwen_rotation_enabled {
                "[*]"
            } else {
                "[ ]"
            };
            lines.push(Line::from(Span::styled(
                format!(
                    "  {} Account rotation (Space){}",
                    rotation_marker,
                    if app.ps.qwen_rotation_enabled {
                        format!("  Accounts: {}", app.ps.qwen_rotation_count)
                    } else {
                        String::new()
                    }
                ),
                Style::default().fg(Color::Cyan),
            )));
            lines.push(Line::from(""));
        }

        // "Authenticated" shortcut — only when NOT mid-rotation
        if app.ps.has_existing_key
            && !app.ps.qwen_rotation_enabled
            && matches!(
                app.ps.qwen_device_flow_status,
                QwenDeviceFlowStatus::Idle | QwenDeviceFlowStatus::Complete
            )
        {
            lines.push(Line::from(Span::styled(
                "  ● Authenticated with Qwen",
                Style::default().fg(Color::Green),
            )));
        } else {
            match &app.ps.qwen_device_flow_status {
                QwenDeviceFlowStatus::Idle => {
                    let persisted = app.ps.qwen_persisted_account_count();
                    if app.ps.qwen_rotation_enabled && persisted >= 2 {
                        // Accounts exist but count changed — show current + needed
                        let needed = app.ps.qwen_rotation_count.saturating_sub(persisted);
                        if needed > 0 {
                            lines.push(Line::from(Span::styled(
                                format!(
                                    "  ● {} accounts configured, {} more needed — press Enter",
                                    persisted, needed
                                ),
                                Style::default().fg(Color::Yellow),
                            )));
                        } else {
                            lines.push(Line::from(Span::styled(
                                format!("  ● {} Qwen accounts configured for rotation!", persisted),
                                Style::default()
                                    .fg(Color::Green)
                                    .add_modifier(Modifier::BOLD),
                            )));
                        }
                    } else {
                        lines.push(Line::from(Span::styled(
                            "  Free tier: 60 req/min, 1000 req/day",
                            Style::default()
                                .fg(Color::DarkGray)
                                .add_modifier(Modifier::ITALIC),
                        )));
                    }
                }
                QwenDeviceFlowStatus::WaitingForUser { verification_uri } => {
                    if !verification_uri.is_empty() {
                        lines.push(Line::from(Span::styled(
                            format!("  1. Open: {}", verification_uri),
                            Style::default().fg(BRAND_BLUE).add_modifier(Modifier::BOLD),
                        )));
                    } else {
                        lines.push(Line::from(Span::styled(
                            "  1. Requesting device code...",
                            Style::default().fg(BRAND_BLUE),
                        )));
                    }
                    if let Some(ref code) = app.ps.qwen_user_code {
                        lines.push(Line::from(Span::styled(
                            format!("  2. Enter code: {}", code),
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        )));
                    }
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "  Waiting for authorization...",
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    )));
                }
                QwenDeviceFlowStatus::Complete => {
                    lines.push(Line::from(Span::styled(
                        "  ● Authenticated successfully!",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )));
                }
                QwenDeviceFlowStatus::Failed(err) => {
                    lines.push(Line::from(Span::styled(
                        format!("  ✗ {}", err),
                        Style::default().fg(Color::Red),
                    )));
                    lines.push(Line::from(Span::styled(
                        "  Press Enter to try again",
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    )));
                }
                QwenDeviceFlowStatus::RotationStep { current, total } => {
                    lines.push(Line::from(Span::styled(
                        format!("  Authenticating account {} of {}...", current + 1, total),
                        Style::default().fg(BRAND_BLUE).add_modifier(Modifier::BOLD),
                    )));
                    if let Some(ref code) = app.ps.qwen_user_code {
                        lines.push(Line::from(Span::styled(
                            format!("  Enter code: {}", code),
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        )));
                    }
                    lines.push(Line::from(Span::styled(
                        "  Waiting for authorization...",
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    )));
                }
                QwenDeviceFlowStatus::RotationSignout { current, total } => {
                    lines.push(Line::from(Span::styled(
                        format!("  Account {} of {} authenticated!", current, total),
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )));
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "  Sign out of Qwen in your browser, then press Enter",
                        Style::default().fg(BRAND_BLUE).add_modifier(Modifier::BOLD),
                    )));
                }
                QwenDeviceFlowStatus::RotationComplete => {
                    lines.push(Line::from(Span::styled(
                        format!(
                            "  ● {} Qwen accounts configured for rotation!",
                            app.ps.qwen_rotation_count
                        ),
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )));
                }
            }
        }
        lines.push(Line::from(""));
    }

    // API Key field (field 1 for non-Custom, field 2 for Custom; field 2 for zhipu since field 1 = endpoint type)
    // CLI providers have no API key — skip entirely.
    // Qwen native (idx 10) handled above via OAuth device flow.
    let is_cli_provider = app.ps.is_cli();
    if !is_cli_provider && provider_idx != 10 {
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
    } else if provider_idx == 10 {
        // Qwen native — field 1 is auth/rotation, not API key
        match focused_field {
            0 => vec![
                ("[↑/↓]", "Select"),
                ("[Enter]", "Next"),
                ("[Tab]", "Skip to Model"),
            ],
            1 => {
                let mut hints = vec![("[Space]", "Rotate")];
                if app.ps.qwen_rotation_enabled {
                    hints.push(("[2-9]", "#"));
                }
                hints.push((
                    "[Enter]",
                    if app.ps.has_existing_key {
                        "Next"
                    } else {
                        "Sign In"
                    },
                ));
                hints.push(("[Esc]", "Back"));
                hints
            }
            2 => vec![
                ("[Type]", "Filter"),
                ("[↑/↓]", "Select"),
                ("[Enter]", "Confirm"),
            ],
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

/// Estimate cost for a model/token pair using the same pricing table as the provider.
/// Used for old sessions that have token_count but zero cost stored.
/// Pricing per million tokens (verified via OpenRouter API 2026-02-25).
fn estimate_cost_from_tokens(model: &str, token_count: i64) -> Option<f64> {
    // Use a fresh load (not the OnceLock) so UI reflects live pricing file edits
    // and avoids stale cache issues with providers like MiniMax.
    crate::pricing::PricingConfig::load().estimate_cost(model, token_count)
}

/// Render the usage stats dialog (centered overlay)
pub(super) fn render_usage_dialog(f: &mut Frame, app: &App, area: Rect) {
    // ── Current session stats ──────────────────────────────────────────────
    let session_name = app
        .current_session
        .as_ref()
        .and_then(|s| s.title.as_deref())
        .unwrap_or("New Session");

    let model = app
        .current_session
        .as_ref()
        .and_then(|s| s.model.clone())
        .unwrap_or_else(|| app.provider_model());

    let message_count = app.messages.len();
    let cur_tokens = app.total_tokens();
    // If stored cost is zero but we have tokens, estimate from pricing table.
    // This covers sessions started before pricing was fixed or mid-session on first run.
    let (cur_cost, cur_cost_estimated) = {
        let stored = app.total_cost();
        if stored > 0.0 {
            (stored, false)
        } else if cur_tokens > 0 {
            (
                estimate_cost_from_tokens(&model, cur_tokens as i64).unwrap_or(0.0),
                true,
            )
        } else {
            (0.0, false)
        }
    };

    // ── All-time stats from usage ledger (survives session deletes) ──────
    let total_sessions = app.sessions.len();
    let all_tokens: i64 = app.usage_ledger_stats.iter().map(|s| s.total_tokens).sum();
    let all_cost: f64 = app.usage_ledger_stats.iter().map(|s| s.total_cost).sum();

    // For display: use ledger stats directly (already sorted by cost desc)
    struct ModelStats {
        tokens: i64,
        cost: f64,
        estimated: bool,
    }
    let model_entries: Vec<(&str, ModelStats)> = app
        .usage_ledger_stats
        .iter()
        .map(|s| {
            let cost = if s.total_cost > 0.0 {
                s.total_cost
            } else if s.total_tokens > 0 {
                estimate_cost_from_tokens(&s.model, s.total_tokens).unwrap_or(0.0)
            } else {
                0.0
            };
            let estimated = s.total_cost == 0.0 && s.total_tokens > 0;
            (
                s.model.as_str(),
                ModelStats {
                    tokens: s.total_tokens,
                    cost,
                    estimated,
                },
            )
        })
        .collect();
    let any_estimated = model_entries.iter().any(|(_, s)| s.estimated);

    // ── Build lines ────────────────────────────────────────────────────────
    let label_style = Style::default().fg(Color::DarkGray);
    let value_style = Style::default()
        .fg(Color::Reset)
        .add_modifier(Modifier::BOLD);
    let dim_style = Style::default().fg(Color::DarkGray);
    let header_style = Style::default()
        .fg(Color::Rgb(120, 120, 120))
        .add_modifier(Modifier::BOLD);
    let est_style = Style::default().fg(Color::Rgb(215, 100, 20));

    let fmt_tokens = |t: i64| -> String {
        if t >= 1_000_000 {
            format!("{:.1}M", t as f64 / 1_000_000.0)
        } else if t >= 1_000 {
            format!("{:.0}K", t as f64 / 1_000.0)
        } else {
            format!("{}", t)
        }
    };

    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(vec![Span::styled("  ── Current Session ──", header_style)]),
        Line::from(vec![
            Span::styled("  Session:  ", label_style),
            Span::styled(session_name.to_string(), value_style),
        ]),
        Line::from(vec![
            Span::styled("  Model:    ", label_style),
            Span::styled(model.to_string(), value_style),
        ]),
        Line::from(vec![
            Span::styled("  Messages: ", label_style),
            Span::styled(format!("{}", message_count), value_style),
        ]),
        Line::from(vec![
            Span::styled("  Tokens:   ", label_style),
            Span::styled(fmt_tokens(cur_tokens as i64), value_style),
        ]),
        Line::from(vec![
            Span::styled("  Cost:     ", label_style),
            Span::styled(
                if cur_cost_estimated {
                    format!("~${:.4}", cur_cost)
                } else {
                    format!("${:.4}", cur_cost)
                },
                if cur_cost_estimated {
                    est_style
                } else {
                    value_style
                },
            ),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled("  ── All Sessions ──", header_style)]),
        Line::from(vec![
            Span::styled("  Sessions: ", label_style),
            Span::styled(format!("{}", total_sessions), value_style),
            Span::styled(format!("  Tokens: {}", fmt_tokens(all_tokens)), dim_style),
        ]),
    ];

    // Per-model breakdown
    for (model_name, stats) in &model_entries {
        let cost_str = if stats.estimated {
            format!("~${:.2}", stats.cost)
        } else {
            format!("${:.2}", stats.cost)
        };
        let short_model = if model_name.len() > 20 {
            format!("{}…", model_name.chars().take(19).collect::<String>())
        } else {
            model_name.to_string()
        };
        lines.push(Line::from(vec![
            Span::styled("    ", dim_style),
            Span::styled(format!("{:<21}", short_model), value_style),
            if stats.estimated {
                Span::styled(format!("{:>8}", cost_str), est_style)
            } else {
                Span::styled(format!("{:>8}", cost_str), value_style)
            },
            Span::styled(format!("  ({})", fmt_tokens(stats.tokens)), dim_style),
        ]));
    }

    // Total
    let total_str = if any_estimated {
        format!("~${:.2}", all_cost)
    } else {
        format!("${:.2}", all_cost)
    };
    lines.push(Line::from(vec![
        Span::styled("  Total:    ", label_style),
        if any_estimated {
            Span::styled(total_str, est_style)
        } else {
            Span::styled(total_str, value_style)
        },
    ]));

    if any_estimated {
        lines.push(Line::from(vec![Span::styled(
            "  ~ = estimated (80/20 token split)",
            Style::default().fg(Color::DarkGray),
        )]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "         [Esc] Close",
        Style::default().fg(Color::DarkGray),
    )]));

    // ── Compute dialog size dynamically ───────────────────────────────────
    let content_lines = lines.len() as u16;
    let dialog_height = (content_lines + 2).min(area.height.saturating_sub(4));
    let dialog_width = 56u16.min(area.width.saturating_sub(4));

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

    f.render_widget(Clear, dialog_area);
    let dialog = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(120, 120, 120)))
            .title(Span::styled(
                " Usage Stats ",
                Style::default()
                    .fg(Color::Rgb(120, 120, 120))
                    .add_modifier(Modifier::BOLD),
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
