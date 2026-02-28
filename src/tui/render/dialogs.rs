//! Dialog rendering
//!
//! File picker, directory picker, model selector, usage dialog, and restart dialog.

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
            Style::default().fg(Color::White)
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
        Span::styled(" Navigate  ", Style::default().fg(Color::White)),
        Span::styled(
            "[Enter]",
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Select  ", Style::default().fg(Color::White)),
        Span::styled(
            "[Esc]",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Cancel", Style::default().fg(Color::White)),
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
        Span::styled(" Navigate  ", Style::default().fg(Color::White)),
        Span::styled(
            "[Enter]",
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Open  ", Style::default().fg(Color::White)),
        Span::styled(
            "[Space/Tab]",
            Style::default()
                .fg(Color::Rgb(50, 205, 50))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Select here  ", Style::default().fg(Color::White)),
        Span::styled(
            "[Esc]",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Cancel", Style::default().fg(Color::White)),
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

    const BRAND_BLUE: Color = Color::Rgb(120, 120, 120);
    const BRAND_GOLD: Color = Color::Rgb(215, 100, 20);

    let focused_field = app.model_selector_focused_field; // 0=provider, 1=api_key, 2=model
    let provider_idx = app.model_selector_provider_selected;
    let selected_provider = &PROVIDERS[provider_idx];

    // Get models from fetched list ONLY - never use static fallback
    // If no fetched models, show message to re-select provider to fetch
    let display_models: Vec<&str> = app
        .model_selector_models
        .iter()
        .map(|s| s.as_ref())
        .collect();

    // If no models fetched yet, show placeholder
    if display_models.is_empty() {
        // Don't show static fallback - tell user to confirm to fetch
    }

    let model_count = display_models.len();
    let current_model = app
        .current_session
        .as_ref()
        .and_then(|s| s.model.clone())
        .unwrap_or_else(|| app.provider_model());

    let dialog_height = (model_count as u16 + 20).min(area.height.saturating_sub(4));
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

    // Provider list (like onboarding)
    for (i, provider) in PROVIDERS.iter().enumerate() {
        let selected = i == provider_idx;
        let focused = focused_field == 0;

        let prefix = if selected && focused { " > " } else { "   " };
        let marker = if selected { "[*]" } else { "[ ]" };

        // For custom provider with a name set, show the name instead of generic label
        let label = if i == 5 && !app.model_selector_custom_name.is_empty() {
            app.model_selector_custom_name.clone()
        } else {
            provider.name.to_string()
        };

        lines.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(BRAND_GOLD)),
            Span::styled(
                marker,
                Style::default().fg(if selected {
                    BRAND_GOLD
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(
                format!(" {}", label),
                Style::default()
                    .fg(if selected {
                        Color::White
                    } else {
                        Color::DarkGray
                    })
                    .add_modifier(if selected {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
        ]));
    }

    lines.push(Line::from(""));

    let is_custom = provider_idx == 5; // Custom provider index

    // For Custom provider: show Base URL field first (field 1), then API Key (field 2)
    // For others: show API Key only (field 1)
    if is_custom {
        // Base URL field (field 1 for Custom)
        let base_focused = focused_field == 1;
        let base_display = if app.model_selector_base_url.is_empty() {
            "http://localhost:1234/v1".to_string()
        } else {
            app.model_selector_base_url.clone()
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
                    Color::White
                } else {
                    Color::Green
                }),
            ),
        ]));
        lines.push(Line::from(""));
    }

    // API Key field (field 1 for non-Custom, field 2 for Custom)
    let key_focused = (focused_field == 1 && !is_custom) || (focused_field == 2 && is_custom);
    let key_label = selected_provider.key_label;

    let has_existing_key = app.model_selector_has_existing_key;
    let has_user_key = !app.model_selector_api_key.is_empty();

    let (masked_key, key_hint) = if has_user_key {
        // User typed a new key - show asterisks for what they typed
        (
            "*".repeat(app.model_selector_api_key.len().min(30)),
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
                Color::White
            } else {
                Color::Green
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

    // Model selection (field 2 for non-Custom, field 3 for Custom)
    let model_focused = (focused_field == 2 && !is_custom) || (focused_field == 3 && is_custom);
    const MAX_VISIBLE_MODELS: usize = 8;

    if is_custom {
        // Custom provider: free-text model name input (no filter/search)
        let model_cursor = if model_focused { "█" } else { "" };
        let model_display = if app.model_selector_custom_model.is_empty() {
            format!("enter model name (e.g. gpt-4o){}", model_cursor)
        } else {
            format!("{}{}", app.model_selector_custom_model, model_cursor)
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
                    Color::White
                } else if app.model_selector_custom_model.is_empty() {
                    Color::DarkGray
                } else {
                    Color::Green
                }),
            ),
        ]));
    } else {
        // Non-custom: filter/search model list
        if model_focused {
            let filter_cursor = if model_focused { "█" } else { "" };
            let filter_display = if app.model_selector_filter.is_empty() {
                format!("  / filter{}", filter_cursor)
            } else {
                format!("  / {}{}", app.model_selector_filter, filter_cursor)
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
        let safe_selected = app.model_selector_selected.min(max_sel);
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
                Style::default().fg(Color::White)
            };

            let suffix = if active { " (active)" } else { "" };

            lines.push(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(*model, style),
                Span::styled(suffix, Style::default().fg(Color::DarkGray)),
            ]));
        }
    }

    // Custom provider: name identifier field (field 4 — last before save)
    if is_custom {
        let name_focused = focused_field == 4;
        let name_cursor = if name_focused { "█" } else { "" };
        let name_display = if app.model_selector_custom_name.is_empty() {
            format!("enter identifier (e.g. nvidia, kimi){}", name_cursor)
        } else {
            format!("{}{}", app.model_selector_custom_name, name_cursor)
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
                    Color::White
                } else if app.model_selector_custom_name.is_empty() {
                    Color::DarkGray
                } else {
                    Color::Green
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
            3 => vec![("[Type]", "Model name"), ("[Enter]", "Confirm")],
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
    for (key, action) in help_text {
        help_spans.push(Span::styled(
            key,
            Style::default()
                .fg(Color::Rgb(215, 100, 20))
                .add_modifier(Modifier::BOLD),
        ));
        help_spans.push(Span::styled(
            format!("{}  ", action),
            Style::default().fg(Color::White),
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

    // ── All-time stats grouped by model ───────────────────────────────────
    // Group sessions by model, sum token_count and total_cost.
    // For sessions with zero cost but non-zero tokens, estimate cost.
    use std::collections::HashMap;
    struct ModelStats {
        sessions: usize,
        tokens: i64,
        cost: f64,
        estimated: bool, // true if any session used estimation
    }
    let mut by_model: HashMap<String, ModelStats> = HashMap::new();

    for s in &app.sessions {
        // Skip empty sessions — no model set yet (new session, no messages sent)
        if s.token_count == 0 && s.total_cost == 0.0 {
            continue;
        }
        // Sessions created before model was persisted won't have a model field.
        // Fall back to the current session's model so they don't bucket under "unknown".
        let model_key = s
            .model
            .clone()
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| {
                app.current_session
                    .as_ref()
                    .and_then(|cs| cs.model.clone())
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(|| app.provider_model().to_string())
            });
        let entry = by_model.entry(model_key.clone()).or_insert(ModelStats {
            sessions: 0,
            tokens: 0,
            cost: 0.0,
            estimated: false,
        });
        entry.sessions += 1;
        entry.tokens += s.token_count as i64;

        if s.total_cost > 0.0 {
            entry.cost += s.total_cost;
        } else if s.token_count > 0 {
            // Cost not stored — estimate from token count
            if let Some(est) = estimate_cost_from_tokens(&model_key, s.token_count as i64) {
                entry.cost += est;
                entry.estimated = true;
            }
        }
    }

    let total_sessions = app.sessions.len();
    let all_tokens: i64 = by_model.values().map(|v| v.tokens).sum();
    let all_cost: f64 = by_model.values().map(|v| v.cost).sum();
    let any_estimated = by_model.values().any(|v| v.estimated);

    // Sort models by cost descending
    let mut model_entries: Vec<(&String, &ModelStats)> = by_model.iter().collect();
    model_entries.sort_by(|a, b| {
        b.1.cost
            .partial_cmp(&a.1.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // ── Build lines ────────────────────────────────────────────────────────
    let label_style = Style::default().fg(Color::DarkGray);
    let value_style = Style::default()
        .fg(Color::White)
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
            format!("{}…", &model_name[..19])
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
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  Restart with new binary?"),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  [Enter] ",
                Style::default()
                    .fg(Color::Green)
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
            .border_style(Style::default().fg(Color::Green)),
    );
    f.render_widget(dialog, dialog_area);
}
