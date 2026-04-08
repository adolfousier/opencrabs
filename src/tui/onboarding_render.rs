//! Onboarding Wizard Rendering
//!
//! Render functions for each step of the onboarding wizard.

use super::onboarding::{
    AuthField, BrainField, CHANNEL_NAMES, ChannelTestStatus, DiscordField, HealthStatus,
    ImageField, OnboardingStep, OnboardingWizard, PROVIDERS, SlackField, TelegramField,
    TrelloField, WizardMode,
};
use super::provider_selector::CUSTOM_PROVIDER_IDX;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

/// Main color palette (matches existing OpenCrabs theme)
const BRAND_BLUE: Color = Color::Rgb(120, 120, 120);
const BRAND_GOLD: Color = Color::Rgb(215, 100, 20);
const ACCENT_GOLD: Color = Color::Rgb(215, 100, 20);

/// Render the entire onboarding wizard
pub fn render_onboarding(f: &mut Frame, wizard: &OnboardingWizard) {
    let area = f.area();

    // Build wizard content FIRST so we know the actual height
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Header
    let step = wizard.step;
    if step != OnboardingStep::Complete && !wizard.quick_jump {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            render_progress_dots(&step),
            Style::default().fg(BRAND_BLUE),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            step.title().to_string(),
            Style::default().fg(BRAND_GOLD).add_modifier(Modifier::BOLD),
        )));
        // Wrap subtitle so it never truncates
        let subtitle_style = Style::default().fg(Color::DarkGray);
        for chunk in wrap_text(step.subtitle(), 54) {
            lines.push(Line::from(Span::styled(chunk, subtitle_style)));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(""));
    } else if wizard.quick_jump {
        // Top padding for doctor/deep-link mode (no header, just spacing)
        lines.push(Line::from(""));
    }

    let header_end = lines.len();

    // Step-specific content; returns a focused-line hint for scrolling
    let focused_line: usize = match step {
        OnboardingStep::ProviderAuth => render_provider_auth(&mut lines, wizard),
        OnboardingStep::Channels => render_channels(&mut lines, wizard),
        OnboardingStep::TelegramSetup => render_telegram_setup(&mut lines, wizard),
        OnboardingStep::DiscordSetup => render_discord_setup(&mut lines, wizard),
        OnboardingStep::WhatsAppSetup => render_whatsapp_setup(&mut lines, wizard),
        OnboardingStep::SlackSetup => render_slack_setup(&mut lines, wizard),
        OnboardingStep::TrelloSetup => render_trello_setup(&mut lines, wizard),
        other => {
            match other {
                OnboardingStep::ModeSelect => render_mode_select(&mut lines, wizard),
                OnboardingStep::Workspace => render_workspace(&mut lines, wizard),
                OnboardingStep::VoiceSetup => render_voice_setup(&mut lines, wizard),
                OnboardingStep::ImageSetup => render_image_setup(&mut lines, wizard),
                OnboardingStep::Daemon => render_daemon(&mut lines, wizard),
                OnboardingStep::HealthCheck => render_health_check(&mut lines, wizard),
                OnboardingStep::BrainSetup => render_brain_setup(&mut lines, wizard),
                OnboardingStep::Complete => render_complete(&mut lines, wizard),
                _ => unreachable!(),
            }
            0
        }
    };

    // Error message
    if let Some(ref err) = wizard.error_message {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  ! {}", err),
            Style::default().fg(Color::Red),
        )));
    }

    // Navigation footer
    if step != OnboardingStep::Complete {
        lines.push(Line::from(""));

        let is_channels = step == OnboardingStep::Channels;
        let is_channel_sub = matches!(
            step,
            OnboardingStep::TelegramSetup
                | OnboardingStep::DiscordSetup
                | OnboardingStep::WhatsAppSetup
                | OnboardingStep::SlackSetup
                | OnboardingStep::TrelloSetup
        );
        let esc_label = if wizard.quick_jump { "Exit" } else { "Back" };

        let mut footer: Vec<Span<'static>> = vec![
            Span::styled(
                " [Esc] ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{}  ", esc_label),
                Style::default().fg(Color::White),
            ),
        ];

        if is_channels {
            // Channels list: Space toggles, Enter opens setup, arrow keys navigate
            footer.push(Span::styled(
                "[Space] ",
                Style::default().fg(BRAND_BLUE).add_modifier(Modifier::BOLD),
            ));
            footer.push(Span::styled("Toggle  ", Style::default().fg(Color::White)));
            footer.push(Span::styled(
                "[Enter] ",
                Style::default()
                    .fg(ACCENT_GOLD)
                    .add_modifier(Modifier::BOLD),
            ));
            footer.push(Span::styled("Setup", Style::default().fg(Color::White)));
        } else if is_channel_sub {
            // Channel setup screens: tab nav + editing hints
            footer.push(Span::styled(
                "[Tab] ",
                Style::default().fg(BRAND_BLUE).add_modifier(Modifier::BOLD),
            ));
            footer.push(Span::styled("Next  ", Style::default().fg(Color::White)));
            footer.push(Span::styled(
                "[←→] ",
                Style::default().fg(BRAND_BLUE).add_modifier(Modifier::BOLD),
            ));
            footer.push(Span::styled("Cursor  ", Style::default().fg(Color::White)));
            footer.push(Span::styled(
                "[Enter] ",
                Style::default()
                    .fg(ACCENT_GOLD)
                    .add_modifier(Modifier::BOLD),
            ));
            footer.push(Span::styled("Confirm", Style::default().fg(Color::White)));
        } else if step == OnboardingStep::HealthCheck {
            footer.push(Span::styled(
                "[Enter] ",
                Style::default()
                    .fg(ACCENT_GOLD)
                    .add_modifier(Modifier::BOLD),
            ));
            if wizard.health_complete {
                footer.push(Span::styled("Re-check", Style::default().fg(Color::White)));
            } else {
                footer.push(Span::styled("Check", Style::default().fg(Color::White)));
            }
        } else {
            // All other steps: Tab/Shift+Tab field nav + Enter confirm
            if step != OnboardingStep::ModeSelect {
                footer.push(Span::styled(
                    "[Tab] ",
                    Style::default().fg(BRAND_BLUE).add_modifier(Modifier::BOLD),
                ));
                footer.push(Span::styled(
                    "Next Field  ",
                    Style::default().fg(Color::White),
                ));
            }
            footer.push(Span::styled(
                "[Enter] ",
                Style::default()
                    .fg(ACCENT_GOLD)
                    .add_modifier(Modifier::BOLD),
            ));
            footer.push(Span::styled("Confirm", Style::default().fg(Color::White)));
        }

        lines.push(Line::from(footer));
    }

    // Bottom padding
    lines.push(Line::from(""));

    // --- Layout calculations ---
    let box_width = 64u16.min(area.width.saturating_sub(4));
    let inner_width = box_width.saturating_sub(2) as usize; // inside borders

    // The header occupies lines 0..header_end (progress dots, title, subtitle).
    // These lines AND the footer/empty lines get centered.
    // Step-specific content lines (radio buttons, fields, descriptions) stay
    // left-aligned as a group so they don't drift relative to each other.

    // Find where the footer starts (the nav line near the bottom).
    // The footer only exists on non-Complete steps: empty separator + nav line + bottom padding.
    let footer_start: usize = if step != OnboardingStep::Complete && lines.len() >= 3 {
        lines.len() - 3 // empty separator, footer line, bottom padding
    } else {
        lines.len() // no footer to center separately
    };

    // Center the step content block as a whole: find max width of content
    // lines and add uniform left padding to shift the whole block to center.
    let content_max_width: usize = lines[header_end..footer_start]
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|s| {
                    use unicode_width::UnicodeWidthStr;
                    s.content.width()
                })
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0);
    let content_pad = if content_max_width > 0 && content_max_width < inner_width {
        (inner_width - content_max_width) / 2
    } else {
        0
    };

    let centered_lines: Vec<Line<'static>> = lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            let line_width: usize = line
                .spans
                .iter()
                .map(|s| {
                    use unicode_width::UnicodeWidthStr;
                    s.content.width()
                })
                .sum();

            if line_width == 0 {
                return line; // empty lines stay empty
            }

            if i < header_end || i >= footer_start {
                // Header and footer: center each line independently
                if line_width >= inner_width {
                    line
                } else {
                    let pad = (inner_width - line_width) / 2;
                    let mut spans = vec![Span::raw(" ".repeat(pad))];
                    spans.extend(line.spans);
                    Line::from(spans)
                }
            } else {
                // Step content: uniform left padding so the block stays aligned
                if content_pad > 0 {
                    let mut spans = vec![Span::raw(" ".repeat(content_pad))];
                    spans.extend(line.spans);
                    Line::from(spans)
                } else {
                    line
                }
            }
        })
        .collect();

    // Calculate actual content height: lines + 2 for top/bottom border
    let content_height = (centered_lines.len() as u16).saturating_add(2);
    // Clamp to available area
    let box_height = content_height.min(area.height.saturating_sub(2));
    // Inner visible rows (no borders) — used for scroll calculation
    let visible_rows = box_height.saturating_sub(2) as usize;
    // For ProviderAuth: scroll so the focused element stays visible,
    // but always keep at least 1 blank line at top for padding.
    let scroll_offset: u16 = if focused_line > 2 && centered_lines.len() > visible_rows {
        let target = focused_line.saturating_sub(2);
        let max_scroll = centered_lines.len().saturating_sub(visible_rows);
        // Never scroll past line 1 so the top padding line (index 0) stays visible
        let clamped = target.min(max_scroll);
        // Keep at least 1 line of top padding visible
        if clamped > 0 {
            clamped.saturating_sub(0) as u16
        } else {
            0
        }
    } else {
        0
    };

    // Center the wizard box on screen using Flex::Center
    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .flex(Flex::Center)
        .constraints([Constraint::Length(box_height)])
        .split(area);

    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .flex(Flex::Center)
        .constraints([Constraint::Length(box_width)])
        .split(v_chunks[0]);

    let wizard_area = h_chunks[0];

    let title_string = if step == OnboardingStep::Complete {
        " OpenCrabs Setup Complete ".to_string()
    } else if wizard.quick_jump {
        format!(" {} ", step.title())
    } else {
        format!(
            " OpenCrabs Setup ({}/{}) ",
            step.number(),
            OnboardingStep::total()
        )
    };

    let title_alignment = if wizard.quick_jump {
        Alignment::Center
    } else {
        Alignment::Left
    };

    let paragraph = Paragraph::new(centered_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BRAND_BLUE))
                .title(Span::styled(
                    title_string,
                    Style::default().fg(BRAND_BLUE).add_modifier(Modifier::BOLD),
                ))
                .title_alignment(title_alignment),
        )
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: false });
    // Only apply scroll when needed — scroll((0,0)) can interact with Wrap
    let paragraph = if scroll_offset > 0 {
        paragraph.scroll((scroll_offset, 0))
    } else {
        paragraph
    };

    f.render_widget(paragraph, wizard_area);

    // WhatsApp QR popup — rendered as a centered overlay with white bg so the
    // QR modules have the contrast needed to be scannable by a phone camera.
    if wizard.step == OnboardingStep::WhatsAppSetup
        && let Some(ref qr_text) = wizard.whatsapp_qr_text
    {
        render_whatsapp_qr_popup(f, qr_text, area);
    }
}

/// Render the WhatsApp QR code as a centered full-screen popup with white
/// background and black foreground so the code is always scannable.
fn render_whatsapp_qr_popup(f: &mut Frame, qr_text: &str, area: Rect) {
    use unicode_width::UnicodeWidthStr;
    let qr_lines: Vec<&str> = qr_text.lines().collect();
    let qr_w = qr_lines.iter().map(|l| l.width()).max().unwrap_or(0) as u16;
    let qr_h = qr_lines.len() as u16;

    // popup = QR + 2 border + 2 header rows (instruction + blank) + 1 footer
    let popup_w = (qr_w + 4).min(area.width);
    let popup_h = (qr_h + 5).min(area.height);

    let x = area.x + area.width.saturating_sub(popup_w) / 2;
    let y = area.y + area.height.saturating_sub(popup_h) / 2;
    let popup_area = Rect {
        x,
        y,
        width: popup_w,
        height: popup_h,
    };

    // Build content lines — white block chars on dark bg (inverted QR, scannable by phone)
    let qr_style = Style::default().fg(Color::White).bg(Color::Rgb(18, 18, 18));
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(qr_h as usize + 3);
    lines.push(Line::from(Span::styled(
        " Open WhatsApp › Linked Devices › Link a Device ",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
    for qr_line in qr_lines {
        lines.push(Line::from(Span::styled(
            format!("  {}  ", qr_line),
            qr_style,
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " Waiting for scan... ",
        Style::default().fg(BRAND_GOLD),
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(120, 120, 120)))
        .style(Style::default().bg(Color::Rgb(18, 18, 18)));

    let para = Paragraph::new(lines)
        .block(block)
        .alignment(Alignment::Center);

    f.render_widget(ratatui::widgets::Clear, popup_area);
    f.render_widget(para, popup_area);
}

/// Render progress dots (filled for completed, hollow for remaining)
fn render_progress_dots(step: &OnboardingStep) -> String {
    let current = step.number();
    let total = OnboardingStep::total();
    (1..=total)
        .map(|i| if i <= current { "●" } else { "○" })
        .collect::<Vec<_>>()
        .join(" ")
}

// --- Individual step renderers ---
// All functions produce Vec<Line<'static>> by using owned strings throughout.

fn render_mode_select(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    let qs_selected = wizard.mode == WizardMode::QuickStart;

    lines.push(Line::from(vec![
        Span::styled(
            if qs_selected { " > " } else { "   " },
            Style::default().fg(ACCENT_GOLD),
        ),
        Span::styled(
            if qs_selected { "[*]" } else { "[ ]" },
            Style::default().fg(if qs_selected {
                BRAND_GOLD
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            " QuickStart",
            Style::default()
                .fg(if qs_selected {
                    Color::White
                } else {
                    Color::DarkGray
                })
                .add_modifier(if qs_selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        "       Sensible defaults, 4 steps",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));

    let adv_selected = !qs_selected;
    lines.push(Line::from(vec![
        Span::styled(
            if adv_selected { " > " } else { "   " },
            Style::default().fg(ACCENT_GOLD),
        ),
        Span::styled(
            if adv_selected { "[*]" } else { "[ ]" },
            Style::default().fg(if adv_selected {
                BRAND_GOLD
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            " Advanced",
            Style::default()
                .fg(if adv_selected {
                    Color::White
                } else {
                    Color::DarkGray
                })
                .add_modifier(if adv_selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        "       Full control, all 7 steps",
        Style::default().fg(Color::DarkGray),
    )));
}

/// Returns the line index (in `lines`) of the currently focused element —
/// used by `render_onboarding` to scroll the Paragraph and keep it visible.
fn render_provider_auth(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) -> usize {
    let is_custom = wizard.ps.is_custom();
    let mut focused_line: usize = 0;

    // Provider list — 8 static providers (0-7), then existing custom names (9+), then "+ New Custom" (8) last.
    // Visual order: 0-7, then 9+, then 8 (existing customs before add button).
    let display_order = wizard.ps.provider_display_order();
    for &idx in &display_order {
        let selected = idx == wizard.ps.selected_provider;
        let focused = wizard.auth_field == AuthField::Provider;

        let prefix = if selected && focused { " > " } else { "   " };
        let marker = if selected { "[*]" } else { "[ ]" };

        let label = if idx == PROVIDERS.len() - 1 {
            "+ New Custom Provider".to_string()
        } else if idx < PROVIDERS.len() {
            PROVIDERS[idx].name.to_string()
        } else {
            let custom_idx = idx - crate::tui::provider_selector::CUSTOM_INSTANCES_START;
            wizard
                .ps
                .custom_names
                .get(custom_idx)
                .cloned()
                .unwrap_or_else(|| "custom".to_string())
        };

        lines.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(ACCENT_GOLD)),
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

    if is_custom {
        let name_focused = wizard.auth_field == AuthField::CustomName;
        let base_focused = wizard.auth_field == AuthField::CustomBaseUrl;
        let api_key_focused = wizard.auth_field == AuthField::CustomApiKey;
        let model_focused = wizard.auth_field == AuthField::CustomModel;

        // Provider Name field
        let name_display = if wizard.ps.custom_name.is_empty() {
            "enter a name (e.g. nvidia, ollama)".to_string()
        } else {
            wizard.ps.custom_name.clone()
        };
        let cursor = if name_focused { "█" } else { "" };
        lines.push(Line::from(vec![
            Span::styled(
                "  Name:     ",
                Style::default().fg(if name_focused {
                    BRAND_BLUE
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(
                format!("{}{}", name_display, cursor),
                Style::default().fg(if name_focused {
                    Color::White
                } else {
                    Color::DarkGray
                }),
            ),
        ]));

        let base_display = if wizard.ps.base_url.is_empty() {
            "http://localhost:8000/v1".to_string()
        } else {
            wizard.ps.base_url.clone()
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
                    Color::DarkGray
                }),
            ),
        ]));

        // API Key field (optional for custom providers)
        let has_existing = wizard.ps.has_existing_key_sentinel();
        let key_display = if wizard.ps.api_key_input.is_empty() {
            "optional".to_string()
        } else if has_existing {
            "● configured".to_string()
        } else {
            "*".repeat(wizard.ps.api_key_input.len().min(30))
        };
        let cursor = if api_key_focused && !has_existing {
            "█"
        } else {
            ""
        };
        lines.push(Line::from(vec![
            Span::styled(
                "  API Key:  ",
                Style::default().fg(if api_key_focused {
                    BRAND_BLUE
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(
                format!("{}{}", key_display, cursor),
                Style::default().fg(if has_existing {
                    Color::Cyan
                } else if api_key_focused {
                    Color::White
                } else {
                    Color::DarkGray
                }),
            ),
        ]));

        let model_display = if wizard.ps.custom_model.is_empty() {
            "model-name".to_string()
        } else {
            wizard.ps.custom_model.clone()
        };
        let cursor = if model_focused { "█" } else { "" };
        lines.push(Line::from(vec![
            Span::styled(
                "  Model:    ",
                Style::default().fg(if model_focused {
                    BRAND_BLUE
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(
                format!("{}{}", model_display, cursor),
                Style::default().fg(if model_focused {
                    Color::White
                } else {
                    Color::DarkGray
                }),
            ),
        ]));

        // Context Window field
        let cw_focused = wizard.auth_field == AuthField::CustomContextWindow;
        let cw_display = if wizard.ps.context_window.is_empty() {
            "e.g. 128000 (optional)".to_string()
        } else {
            wizard.ps.context_window.clone()
        };
        let cursor = if cw_focused { "█" } else { "" };
        lines.push(Line::from(vec![
            Span::styled(
                "  Context:  ",
                Style::default().fg(if cw_focused {
                    BRAND_BLUE
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(
                format!("{}{}", cw_display, cursor),
                Style::default().fg(if cw_focused {
                    Color::White
                } else {
                    Color::DarkGray
                }),
            ),
        ]));
    } else if wizard.ps.selected_provider == 10 {
        // Qwen native — OAuth device flow
        use crate::tui::onboarding::QwenDeviceFlowStatus;

        if wizard.ps.has_existing_key_sentinel() {
            lines.push(Line::from(Span::styled(
                "  ● Authenticated with Qwen",
                Style::default().fg(Color::Green),
            )));
            lines.push(Line::from(Span::styled(
                "  Press Enter to continue, or re-authenticate below",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )));
        } else {
            match &wizard.qwen_device_flow_status {
                QwenDeviceFlowStatus::Idle => {
                    lines.push(Line::from(Span::styled(
                        "  Free tier: 60 req/min, 1000 req/day",
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    )));
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "  Press Enter to sign in with qwen.ai",
                        Style::default().fg(BRAND_BLUE).add_modifier(Modifier::BOLD),
                    )));
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
                    if let Some(ref code) = wizard.qwen_user_code {
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
            }
        }
    } else if wizard.ps.selected_provider == 2 {
        // GitHub Copilot — OAuth device flow
        use crate::tui::onboarding::GitHubDeviceFlowStatus;

        if wizard.ps.has_existing_key_sentinel() {
            // Already authenticated
            lines.push(Line::from(Span::styled(
                "  ● Authenticated with GitHub Copilot",
                Style::default().fg(Color::Green),
            )));
            lines.push(Line::from(Span::styled(
                "  Press Enter to continue, or re-authenticate below",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )));
        } else {
            match &wizard.github_device_flow_status {
                GitHubDeviceFlowStatus::Idle => {
                    lines.push(Line::from(Span::styled(
                        "  Uses your GitHub Copilot subscription (no API charges)",
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    )));
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "  Press Enter to sign in with GitHub",
                        Style::default().fg(BRAND_BLUE).add_modifier(Modifier::BOLD),
                    )));
                }
                GitHubDeviceFlowStatus::WaitingForUser => {
                    lines.push(Line::from(Span::styled(
                        "  1. Go to: github.com/login/device",
                        Style::default().fg(BRAND_BLUE).add_modifier(Modifier::BOLD),
                    )));
                    if let Some(ref code) = wizard.github_user_code {
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
                GitHubDeviceFlowStatus::Complete => {
                    lines.push(Line::from(Span::styled(
                        "  ● Authenticated successfully!",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )));
                }
                GitHubDeviceFlowStatus::Failed(err) => {
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
            }
        }
    } else {
        // Show help text for selected provider
        let provider = wizard.ps.current_provider();
        for help_line in provider.help_lines {
            lines.push(Line::from(Span::styled(
                format!("  {}", help_line),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )));
        }
        lines.push(Line::from(""));

        // z.ai GLM endpoint type toggle (api vs coding) — BEFORE API key
        if wizard.ps.selected_provider == 6 {
            let et_focused = wizard.auth_field == AuthField::ZhipuEndpointType;
            let api_marker = if wizard.ps.zhipu_endpoint_type == 0 {
                "[*]"
            } else {
                "[ ]"
            };
            let coding_marker = if wizard.ps.zhipu_endpoint_type == 1 {
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
                    Style::default().fg(if et_focused && wizard.ps.zhipu_endpoint_type == 0 {
                        Color::White
                    } else {
                        Color::DarkGray
                    }),
                ),
                Span::styled(
                    format!("{} Coding API", coding_marker),
                    Style::default().fg(if et_focused && wizard.ps.zhipu_endpoint_type == 1 {
                        Color::White
                    } else {
                        Color::DarkGray
                    }),
                ),
            ]));
            lines.push(Line::from(""));
        }

        // CLI providers have no API key — skip the field
        if !wizard.ps.is_cli() {
            let key_focused = wizard.auth_field == AuthField::ApiKey;
            let key_label = provider.key_label;
            let (masked_key, key_hint) = if wizard.ps.has_existing_key_sentinel() {
                (
                    "**************************".to_string(),
                    " (already configured, type to replace)".to_string(),
                )
            } else if wizard.ps.api_key_input.is_empty() {
                (
                    format!("enter your {}", key_label.to_lowercase()),
                    String::new(),
                )
            } else {
                (
                    "*".repeat(wizard.ps.api_key_input.len().min(30)),
                    String::new(),
                )
            };
            let cursor = if key_focused && !wizard.ps.has_existing_key_sentinel() {
                "█"
            } else {
                ""
            };

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
                    Style::default().fg(if wizard.ps.has_existing_key_sentinel() {
                        Color::Cyan
                    } else if key_focused {
                        Color::White
                    } else {
                        Color::DarkGray
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
        }
    }

    // Model selection — shared across all non-custom providers (including GitHub)
    if !is_custom {
        let model_focused = wizard.auth_field == AuthField::Model;
        let model_count = wizard.ps.model_count();
        if model_count > 0 || wizard.ps.models_fetching {
            lines.push(Line::from(""));
            // Record scroll anchor: 2 lines above the Model: label so the key
            // line stays visible as context when scrolling into the model section.
            if model_focused {
                focused_line = lines.len().saturating_sub(1);
            }
            let label = if wizard.ps.models_fetching {
                "  Model: (fetching...)".to_string()
            } else {
                "  Model:".to_string()
            };
            lines.push(Line::from(Span::styled(
                label,
                Style::default().fg(if model_focused {
                    BRAND_BLUE
                } else {
                    Color::DarkGray
                }),
            )));

            const MAX_VISIBLE_MODELS: usize = 8;

            // Helper: render a windowed slice of models, keeping selection visible
            let render_model_window = |lines: &mut Vec<Line<'static>>,
                                       models: &[&str],
                                       selected: usize,
                                       focused: bool| {
                let total = models.len();
                let (start, end) = if total <= MAX_VISIBLE_MODELS {
                    (0, total)
                } else {
                    let half = MAX_VISIBLE_MODELS / 2;
                    let s = selected
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
                for (offset, model) in models[start..end].iter().enumerate() {
                    let i = start + offset;
                    let is_sel = i == selected;
                    let prefix = if is_sel && focused { " > " } else { "   " };
                    let marker = if is_sel { "(*)" } else { "( )" };
                    let label =
                        crate::tui::provider_selector::model_display_label(model).to_string();
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("  {}{} ", prefix, marker),
                            Style::default().fg(if is_sel { ACCENT_GOLD } else { Color::DarkGray }),
                        ),
                        Span::styled(
                            label,
                            Style::default().fg(if is_sel {
                                Color::White
                            } else {
                                Color::DarkGray
                            }),
                        ),
                    ]));
                }
                if end < total {
                    lines.push(Line::from(Span::styled(
                        format!("  ↓ {} more", total - end),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
            };

            if !wizard.ps.models_fetching {
                // Filter input (shown when model field is focused)
                if model_focused {
                    let cursor = "█";
                    let filter_display = if wizard.ps.model_filter.is_empty() {
                        format!("  / type to filter…{}", cursor)
                    } else {
                        format!("  / {}{}", wizard.ps.model_filter, cursor)
                    };
                    lines.push(Line::from(Span::styled(
                        filter_display,
                        Style::default().fg(if wizard.ps.model_filter.is_empty() {
                            Color::DarkGray
                        } else {
                            Color::White
                        }),
                    )));
                }

                let filtered = wizard.ps.filtered_model_names();
                if filtered.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "  no models match".to_string(),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    )));
                } else {
                    render_model_window(lines, &filtered, wizard.ps.selected_model, model_focused);
                }
            }
        }
    }
    focused_line
}

fn render_workspace(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    let path_focused = wizard.focused_field == 0;
    let seed_focused = wizard.focused_field == 1;

    let cursor = if path_focused { "█" } else { "" };
    lines.push(Line::from(vec![
        Span::styled(
            "  Path: ",
            Style::default().fg(if path_focused {
                BRAND_BLUE
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            format!("{}{}", wizard.workspace_path, cursor),
            Style::default().fg(if path_focused {
                Color::White
            } else {
                Color::DarkGray
            }),
        ),
    ]));
    lines.push(Line::from(""));

    lines.push(Line::from(vec![
        Span::styled(
            if seed_focused { " > " } else { "   " },
            Style::default().fg(ACCENT_GOLD),
        ),
        Span::styled(
            if wizard.seed_templates { "[x]" } else { "[ ]" },
            Style::default().fg(if wizard.seed_templates {
                BRAND_GOLD
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            " Seed template files",
            Style::default().fg(if seed_focused {
                Color::White
            } else {
                Color::DarkGray
            }),
        ),
    ]));

    lines.push(Line::from(Span::styled(
        "       SOUL.md, IDENTITY.md, USER.md, ...",
        Style::default().fg(Color::DarkGray),
    )));
}

fn render_channels(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) -> usize {
    lines.push(Line::from(Span::styled(
        "  Pick your channels (Space to toggle):",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));

    let mut focused_line = 0;
    for (i, (name, enabled)) in wizard.channel_toggles.iter().enumerate() {
        let focused = i == wizard.focused_field;
        if focused {
            focused_line = lines.len();
        }
        let prefix = if focused { " > " } else { "   " };
        let marker = if *enabled { "[x]" } else { "[ ]" };
        // Get the description from CHANNEL_NAMES
        let desc = CHANNEL_NAMES.get(i).map(|(_, d)| *d).unwrap_or("");

        lines.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(ACCENT_GOLD)),
            Span::styled(
                marker,
                Style::default().fg(if *enabled {
                    BRAND_GOLD
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(
                format!(" {}", name),
                Style::default()
                    .fg(if focused {
                        Color::White
                    } else {
                        Color::DarkGray
                    })
                    .add_modifier(if focused {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
        ]));
        lines.push(Line::from(Span::styled(
            format!("       {}", desc),
            Style::default().fg(Color::DarkGray),
        )));
    }

    // "Continue" button at the bottom
    let continue_focused = wizard.focused_field >= wizard.channel_toggles.len();
    if continue_focused {
        focused_line = lines.len() + 1; // +1 for the blank line
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            if continue_focused { " > " } else { "   " },
            Style::default().fg(ACCENT_GOLD),
        ),
        Span::styled(
            "Continue →",
            Style::default()
                .fg(if continue_focused {
                    Color::White
                } else {
                    Color::DarkGray
                })
                .add_modifier(if continue_focused {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
    ]));
    lines.push(Line::from(""));
    focused_line
}

fn render_telegram_setup(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) -> usize {
    let base = lines.len();
    // Help text
    lines.push(Line::from(Span::styled(
        "  1. Open Telegram, search @BotFather",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(Span::styled(
        "  2. Send /newbot, follow the prompts",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(Span::styled(
        "  3. Copy the bot token and paste below",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    // Bot token input
    let token_focused = wizard.telegram_field == TelegramField::BotToken;
    let (masked_token, token_hint) = if wizard.has_existing_telegram_token() {
        (
            "**************************".to_string(),
            " (already configured)".to_string(),
        )
    } else if wizard.telegram_token_input.is_empty() {
        ("paste your bot token".to_string(), String::new())
    } else {
        (
            "*".repeat(wizard.telegram_token_input.len().min(30)),
            String::new(),
        )
    };
    let cursor = if token_focused && !wizard.has_existing_telegram_token() {
        "\u{2588}"
    } else {
        ""
    };

    lines.push(Line::from(vec![
        Span::styled(
            "  Bot Token: ",
            Style::default().fg(if token_focused {
                BRAND_BLUE
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            format!("{}{}", masked_token, cursor),
            Style::default().fg(if wizard.has_existing_telegram_token() {
                Color::Cyan
            } else if token_focused {
                Color::White
            } else {
                Color::DarkGray
            }),
        ),
    ]));

    if !token_hint.is_empty() && token_focused {
        lines.push(Line::from(Span::styled(
            format!("  {}", token_hint.trim()),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    // User ID input
    let uid_focused = wizard.telegram_field == TelegramField::UserID;
    let (uid_display, uid_hint) = if wizard.has_existing_telegram_user_id() {
        (
            "**********".to_string(),
            " (already configured)".to_string(),
        )
    } else if wizard.telegram_user_id_input.is_empty() {
        ("your numeric user ID".to_string(), String::new())
    } else {
        (wizard.telegram_user_id_input.clone(), String::new())
    };
    let uid_cursor = if uid_focused && !wizard.has_existing_telegram_user_id() {
        "\u{2588}"
    } else {
        ""
    };

    lines.push(Line::from(vec![
        Span::styled(
            "  User ID:   ",
            Style::default().fg(if uid_focused {
                BRAND_BLUE
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            format!("{}{}", uid_display, uid_cursor),
            Style::default().fg(if wizard.has_existing_telegram_user_id() {
                Color::Cyan
            } else if uid_focused {
                Color::White
            } else {
                Color::DarkGray
            }),
        ),
    ]));

    if !uid_hint.is_empty() && uid_focused {
        lines.push(Line::from(Span::styled(
            format!("  {}", uid_hint.trim()),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Send /start to your bot to get your user ID",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(Span::styled(
        "  User ID is optional — leave empty to allow all users",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));

    lines.push(Line::from(""));
    render_respond_to_selector(
        lines,
        wizard.telegram_respond_to,
        wizard.telegram_field == TelegramField::RespondTo,
    );

    // Test status
    render_channel_test_status(lines, wizard);

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Tab/Shift+Tab: nav fields | \u{2190}\u{2192}: cursor | Ctrl+\u{232b}: clear | Enter: confirm",
        Style::default().fg(Color::DarkGray),
    )));
    // Focused field for scrolling: token=base+4, uid=base+6, respond_to=base+10 (approx)
    let offset = match wizard.telegram_field {
        TelegramField::BotToken => 4,
        TelegramField::UserID => 6,
        TelegramField::RespondTo => lines.len().saturating_sub(base).saturating_sub(4),
    };
    base + offset
}

fn render_discord_setup(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) -> usize {
    let base = lines.len();
    // Help text
    lines.push(Line::from(Span::styled(
        "  1. Go to discord.com/developers/applications",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(Span::styled(
        "  2. Create app > Bot > Copy token",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(Span::styled(
        "  3. Enable Message Content Intent",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    // Bot token input
    let token_focused = wizard.discord_field == DiscordField::BotToken;
    let (masked_token, token_hint) = if wizard.has_existing_discord_token() {
        (
            "**************************".to_string(),
            " (already configured)".to_string(),
        )
    } else if wizard.discord_token_input.is_empty() {
        ("paste your bot token".to_string(), String::new())
    } else {
        (
            "*".repeat(wizard.discord_token_input.len().min(30)),
            String::new(),
        )
    };
    let cursor = if token_focused && !wizard.has_existing_discord_token() {
        "\u{2588}"
    } else {
        ""
    };

    lines.push(Line::from(vec![
        Span::styled(
            "  Bot Token:   ",
            Style::default().fg(if token_focused {
                BRAND_BLUE
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            format!("{}{}", masked_token, cursor),
            Style::default().fg(if wizard.has_existing_discord_token() {
                Color::Cyan
            } else if token_focused {
                Color::White
            } else {
                Color::DarkGray
            }),
        ),
    ]));

    if !token_hint.is_empty() && token_focused {
        lines.push(Line::from(Span::styled(
            format!("  {}", token_hint.trim()),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    // Channel ID input
    let ch_focused = wizard.discord_field == DiscordField::ChannelID;
    let (ch_display, ch_hint) = if wizard.has_existing_discord_channel_id() {
        (
            "**********".to_string(),
            " (already configured)".to_string(),
        )
    } else if wizard.discord_channel_id_input.is_empty() {
        (
            "right-click channel > Copy Channel ID".to_string(),
            String::new(),
        )
    } else {
        (wizard.discord_channel_id_input.clone(), String::new())
    };
    let ch_cursor = if ch_focused && !wizard.has_existing_discord_channel_id() {
        "\u{2588}"
    } else {
        ""
    };

    lines.push(Line::from(vec![
        Span::styled(
            "  Channel ID:  ",
            Style::default().fg(if ch_focused {
                BRAND_BLUE
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            format!("{}{}", ch_display, ch_cursor),
            Style::default().fg(if wizard.has_existing_discord_channel_id() {
                Color::Cyan
            } else if ch_focused {
                Color::White
            } else {
                Color::DarkGray
            }),
        ),
    ]));

    if !ch_hint.is_empty() && ch_focused {
        lines.push(Line::from(Span::styled(
            format!("  {}", ch_hint.trim()),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    // Allowed List input (Discord user ID — who the bot replies to)
    let al_focused = wizard.discord_field == DiscordField::AllowedList;
    let (al_display, al_hint) = if wizard.has_existing_discord_allowed_list() {
        (
            "**********".to_string(),
            " (already configured)".to_string(),
        )
    } else if wizard.discord_allowed_list_input.is_empty() {
        (
            "user ID (optional — empty = reply to all)".to_string(),
            String::new(),
        )
    } else {
        (wizard.discord_allowed_list_input.clone(), String::new())
    };
    let al_cursor = if al_focused && !wizard.has_existing_discord_allowed_list() {
        "\u{2588}"
    } else {
        ""
    };

    lines.push(Line::from(vec![
        Span::styled(
            "  Allowed List: ",
            Style::default().fg(if al_focused {
                BRAND_BLUE
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            format!("{}{}", al_display, al_cursor),
            Style::default().fg(if wizard.has_existing_discord_allowed_list() {
                Color::Cyan
            } else if al_focused {
                Color::White
            } else {
                Color::DarkGray
            }),
        ),
    ]));

    if !al_hint.is_empty() && al_focused {
        lines.push(Line::from(Span::styled(
            format!("  {}", al_hint.trim()),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    lines.push(Line::from(""));
    render_respond_to_selector(
        lines,
        wizard.discord_respond_to,
        wizard.discord_field == DiscordField::RespondTo,
    );

    // Test status
    render_channel_test_status(lines, wizard);

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Tab/Shift+Tab: nav fields | \u{2190}\u{2192}: cursor | Ctrl+\u{232b}: clear | Enter: confirm",
        Style::default().fg(Color::DarkGray),
    )));
    let offset = match wizard.discord_field {
        DiscordField::BotToken => 4,
        DiscordField::ChannelID => 6,
        DiscordField::AllowedList => 8,
        DiscordField::RespondTo => lines.len().saturating_sub(base).saturating_sub(4),
    };
    base + offset
}

fn render_whatsapp_setup(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) -> usize {
    use crate::tui::onboarding::WhatsAppField;
    let base = lines.len();

    // Connection section
    let conn_focused = wizard.whatsapp_field == WhatsAppField::Connection;
    if wizard.whatsapp_connected {
        lines.push(Line::from(Span::styled(
            "  WhatsApp connected!",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
    } else if wizard.whatsapp_qr_text.is_some() {
        // QR is rendered as a full-screen popup overlay — just show a placeholder here
        lines.push(Line::from(Span::styled(
            "  QR code displayed — scan with WhatsApp",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
        lines.push(Line::from(Span::styled(
            "  Waiting for scan...",
            Style::default().fg(BRAND_GOLD),
        )));
    } else if wizard.whatsapp_connecting {
        lines.push(Line::from(Span::styled(
            "  Starting WhatsApp connection...",
            Style::default().fg(Color::DarkGray),
        )));
    } else if let Some(ref err) = wizard.whatsapp_error {
        lines.push(Line::from(Span::styled(
            format!("  Error: {}", err),
            Style::default().fg(Color::Red),
        )));
        lines.push(Line::from(Span::styled(
            "  Logs: ~/.opencrabs/logs/",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));
        if conn_focused {
            lines.push(Line::from(Span::styled(
                "  Press Enter to retry, 'R' to reset session, or 'S' to skip",
                Style::default().fg(Color::DarkGray),
            )));
        }
    } else if conn_focused {
        let session_db = crate::config::opencrabs_home()
            .join("whatsapp")
            .join("session.db");
        if session_db.exists() {
            lines.push(Line::from(Span::styled(
                "  Previously connected  ·  Press R to reset and re-pair",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "  Press Enter to show QR code",
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    lines.push(Line::from(""));

    // Phone allowlist field
    let phone_focused = wizard.whatsapp_field == WhatsAppField::PhoneAllowlist;
    let phone_display = if wizard.has_existing_whatsapp_phone() {
        "**********".to_string()
    } else if wizard.whatsapp_phone_input.is_empty() {
        "+15551234567".to_string()
    } else {
        wizard.whatsapp_phone_input.clone()
    };
    let phone_cursor = if phone_focused && !wizard.has_existing_whatsapp_phone() {
        "\u{2588}"
    } else {
        ""
    };

    lines.push(Line::from(vec![
        Span::styled(
            "  Allowed Phone: ",
            Style::default().fg(if phone_focused {
                BRAND_BLUE
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            format!("{}{}", phone_display, phone_cursor),
            Style::default().fg(if wizard.has_existing_whatsapp_phone() {
                Color::Cyan
            } else if phone_focused {
                Color::White
            } else {
                Color::DarkGray
            }),
        ),
    ]));

    if wizard.has_existing_whatsapp_phone() && phone_focused {
        lines.push(Line::from(Span::styled(
            "  Type a new number to replace, or press Enter to keep existing",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    } else if wizard.whatsapp_phone_input.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Optional — leave empty to allow all numbers",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    // Test status (shown after phone is confirmed and test fires)
    render_channel_test_status(lines, wizard);

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Tab/Shift+Tab: nav fields | \u{2190}\u{2192}: cursor | Ctrl+\u{232b}: clear | Enter: confirm | S: skip",
        Style::default().fg(Color::DarkGray),
    )));
    let offset = match wizard.whatsapp_field {
        WhatsAppField::Connection => 0,
        WhatsAppField::PhoneAllowlist => lines.len().saturating_sub(base).saturating_sub(6),
    };
    base + offset
}

fn render_slack_setup(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) -> usize {
    let base = lines.len();
    // Help text
    lines.push(Line::from(Span::styled(
        "  1. Go to api.slack.com/apps > Create App",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(Span::styled(
        "  2. OAuth > Bot Token Scopes: chat:write, channels:history,",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(Span::styled(
        "     groups:history, im:history, mpim:history, users:read,",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(Span::styled(
        "     files:read, files:write, reactions:write, app_mentions:read",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(Span::styled(
        "  3. Enable Socket Mode > copy App Token (xapp-...)",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(Span::styled(
        "  4. Install App to Workspace > copy Bot Token (xoxb-...)",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    // Bot token input
    let bot_focused = wizard.slack_field == SlackField::BotToken;
    let (masked_bot, bot_hint) = if wizard.has_existing_slack_bot_token() {
        (
            "**************************".to_string(),
            " (already configured)".to_string(),
        )
    } else if wizard.slack_bot_token_input.is_empty() {
        ("xoxb-...".to_string(), String::new())
    } else {
        (
            "*".repeat(wizard.slack_bot_token_input.len().min(30)),
            String::new(),
        )
    };
    let cursor_b = if bot_focused && !wizard.has_existing_slack_bot_token() {
        "\u{2588}"
    } else {
        ""
    };

    lines.push(Line::from(vec![
        Span::styled(
            "  Bot Token: ",
            Style::default().fg(if bot_focused {
                BRAND_BLUE
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            format!("{}{}", masked_bot, cursor_b),
            Style::default().fg(if wizard.has_existing_slack_bot_token() {
                Color::Cyan
            } else if bot_focused {
                Color::White
            } else {
                Color::DarkGray
            }),
        ),
    ]));

    if !bot_hint.is_empty() && bot_focused {
        lines.push(Line::from(Span::styled(
            format!("  {}", bot_hint.trim()),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    // App token input
    let app_focused = wizard.slack_field == SlackField::AppToken;
    let (masked_app, app_hint) = if wizard.has_existing_slack_app_token() {
        (
            "**************************".to_string(),
            " (already configured)".to_string(),
        )
    } else if wizard.slack_app_token_input.is_empty() {
        ("xapp-...".to_string(), String::new())
    } else {
        (
            "*".repeat(wizard.slack_app_token_input.len().min(30)),
            String::new(),
        )
    };
    let cursor_a = if app_focused && !wizard.has_existing_slack_app_token() {
        "\u{2588}"
    } else {
        ""
    };

    lines.push(Line::from(vec![
        Span::styled(
            "  App Token: ",
            Style::default().fg(if app_focused {
                BRAND_BLUE
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            format!("{}{}", masked_app, cursor_a),
            Style::default().fg(if wizard.has_existing_slack_app_token() {
                Color::Cyan
            } else if app_focused {
                Color::White
            } else {
                Color::DarkGray
            }),
        ),
    ]));

    if !app_hint.is_empty() && app_focused {
        lines.push(Line::from(Span::styled(
            format!("  {}", app_hint.trim()),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    // Channel ID input
    let ch_focused = wizard.slack_field == SlackField::ChannelID;
    let (ch_display, ch_hint) = if wizard.has_existing_slack_channel_id() {
        (
            "**********".to_string(),
            " (already configured)".to_string(),
        )
    } else if wizard.slack_channel_id_input.is_empty() {
        ("C12345678".to_string(), String::new())
    } else {
        (wizard.slack_channel_id_input.clone(), String::new())
    };
    let ch_cursor = if ch_focused && !wizard.has_existing_slack_channel_id() {
        "\u{2588}"
    } else {
        ""
    };

    lines.push(Line::from(vec![
        Span::styled(
            "  Channel ID: ",
            Style::default().fg(if ch_focused {
                BRAND_BLUE
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            format!("{}{}", ch_display, ch_cursor),
            Style::default().fg(if wizard.has_existing_slack_channel_id() {
                Color::Cyan
            } else if ch_focused {
                Color::White
            } else {
                Color::DarkGray
            }),
        ),
    ]));

    if !ch_hint.is_empty() && ch_focused {
        lines.push(Line::from(Span::styled(
            format!("  {}", ch_hint.trim()),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    // Allowed List input (Slack user ID — who the bot replies to)
    let al_focused = wizard.slack_field == SlackField::AllowedList;
    let (al_display, al_hint) = if wizard.has_existing_slack_allowed_list() {
        (
            "**********".to_string(),
            " (already configured)".to_string(),
        )
    } else if wizard.slack_allowed_list_input.is_empty() {
        (
            "U12345678 (optional — empty = reply to all)".to_string(),
            String::new(),
        )
    } else {
        (wizard.slack_allowed_list_input.clone(), String::new())
    };
    let al_cursor = if al_focused && !wizard.has_existing_slack_allowed_list() {
        "\u{2588}"
    } else {
        ""
    };

    lines.push(Line::from(vec![
        Span::styled(
            "  Allowed List: ",
            Style::default().fg(if al_focused {
                BRAND_BLUE
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            format!("{}{}", al_display, al_cursor),
            Style::default().fg(if wizard.has_existing_slack_allowed_list() {
                Color::Cyan
            } else if al_focused {
                Color::White
            } else {
                Color::DarkGray
            }),
        ),
    ]));

    if !al_hint.is_empty() && al_focused {
        lines.push(Line::from(Span::styled(
            format!("  {}", al_hint.trim()),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    lines.push(Line::from(""));
    render_respond_to_selector(
        lines,
        wizard.slack_respond_to,
        wizard.slack_field == SlackField::RespondTo,
    );

    // Test status
    render_channel_test_status(lines, wizard);

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Tab/Shift+Tab: nav fields | \u{2190}\u{2192}: cursor | Ctrl+\u{232b}: clear | Enter: confirm",
        Style::default().fg(Color::DarkGray),
    )));
    let offset = match wizard.slack_field {
        SlackField::BotToken => 4,
        SlackField::AppToken => 6,
        SlackField::ChannelID => 8,
        SlackField::AllowedList => 10,
        SlackField::RespondTo => lines.len().saturating_sub(base).saturating_sub(4),
    };
    base + offset
}

/// Render respond_to selector: `  Respond to: [ all ]  dm_only  mention`
/// `selected` = 0..2, `focused` = whether this field has keyboard focus.
fn render_respond_to_selector(lines: &mut Vec<Line<'static>>, selected: usize, focused: bool) {
    const OPTIONS: [&str; 3] = ["all", "dm_only", "mention"];
    let label_style = Style::default().fg(if focused { BRAND_BLUE } else { Color::DarkGray });
    let mut spans: Vec<Span<'static>> = vec![Span::styled("  Respond to: ", label_style)];
    for (i, opt) in OPTIONS.iter().enumerate() {
        let is_sel = i == selected;
        let (prefix, suffix) = if is_sel { ("[", "]") } else { (" ", " ") };
        let style = if is_sel && focused {
            Style::default().fg(BRAND_GOLD).add_modifier(Modifier::BOLD)
        } else if is_sel {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(Span::styled(format!("{}{}{}", prefix, opt, suffix), style));
        if i < OPTIONS.len() - 1 {
            spans.push(Span::styled("  ", Style::default()));
        }
    }
    lines.push(Line::from(spans));
    if focused {
        lines.push(Line::from(Span::styled(
            "  ← → to change",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }
}

/// Render the channel test connection status line (shared by Telegram/Discord/Slack)
fn render_channel_test_status(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    match &wizard.channel_test_status {
        ChannelTestStatus::Idle => {}
        ChannelTestStatus::Testing => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Testing connection...",
                Style::default().fg(BRAND_GOLD),
            )));
        }
        ChannelTestStatus::Success => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Connected! Press Enter to continue",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        ChannelTestStatus::Failed(err) => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("  Error: {}", err),
                Style::default().fg(Color::Red),
            )));
            lines.push(Line::from(Span::styled(
                "  Enter to retry | S to skip",
                Style::default().fg(Color::DarkGray),
            )));
        }
    }
}

fn render_image_setup(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    // Provider row
    lines.push(Line::from(vec![
        Span::styled(
            "  Provider: ".to_string(),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            "[ Google ]".to_string(),
            Style::default().fg(BRAND_GOLD).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "  gemini-3.1-flash-image-preview".to_string(),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            "  🍌 Nano Banana".to_string(),
            Style::default().fg(BRAND_GOLD),
        ),
    ]));
    lines.push(Line::from(""));

    // Contextual key hints
    let hint_text = match wizard.image_field {
        ImageField::VisionToggle | ImageField::GenerationToggle => {
            "  Space / ↑↓ to toggle  ·  Tab / Enter to continue  ·  Esc to go back"
        }
        ImageField::ApiKey => "  Enter to continue  ·  BackTab to go back  ·  Esc to go back",
    };
    lines.push(Line::from(Span::styled(
        hint_text,
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));

    // Vision toggle
    let vision_focused = wizard.image_field == ImageField::VisionToggle;
    lines.push(Line::from(vec![
        Span::styled(
            if vision_focused { " > " } else { "   " },
            Style::default().fg(ACCENT_GOLD),
        ),
        Span::styled(
            if wizard.image_vision_enabled {
                "[x]".to_string()
            } else {
                "[ ]".to_string()
            },
            Style::default().fg(if wizard.image_vision_enabled {
                BRAND_GOLD
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            " Vision Analysis",
            Style::default()
                .fg(if vision_focused {
                    Color::White
                } else {
                    Color::DarkGray
                })
                .add_modifier(if vision_focused {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
        Span::styled(
            "   — analyze images the agent receives",
            Style::default().fg(Color::DarkGray),
        ),
    ]));

    // Generation toggle
    let gen_focused = wizard.image_field == ImageField::GenerationToggle;
    lines.push(Line::from(vec![
        Span::styled(
            if gen_focused { " > " } else { "   " },
            Style::default().fg(ACCENT_GOLD),
        ),
        Span::styled(
            if wizard.image_generation_enabled {
                "[x]".to_string()
            } else {
                "[ ]".to_string()
            },
            Style::default().fg(if wizard.image_generation_enabled {
                BRAND_GOLD
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            " Image Generation",
            Style::default()
                .fg(if gen_focused {
                    Color::White
                } else {
                    Color::DarkGray
                })
                .add_modifier(if gen_focused {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
        Span::styled(
            " — generate images from text prompts",
            Style::default().fg(Color::DarkGray),
        ),
    ]));

    // API key field (only when either is enabled)
    if wizard.image_vision_enabled || wizard.image_generation_enabled {
        lines.push(Line::from(""));

        let key_focused = wizard.image_field == ImageField::ApiKey;
        let (masked_key, key_hint) = if wizard.has_existing_image_key() {
            (
                "**************************".to_string(),
                " (key already set)".to_string(),
            )
        } else if wizard.image_api_key_input.is_empty() {
            (
                "paste key from aistudio.google.com".to_string(),
                String::new(),
            )
        } else {
            (
                "*".repeat(wizard.image_api_key_input.len().min(30)),
                String::new(),
            )
        };
        let cursor = if key_focused && !wizard.has_existing_image_key() {
            "█"
        } else {
            ""
        };

        lines.push(Line::from(vec![
            Span::styled(
                "  Google API Key: ",
                Style::default().fg(if key_focused {
                    BRAND_BLUE
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(
                format!("{}{}", masked_key, cursor),
                Style::default().fg(if wizard.has_existing_image_key() {
                    Color::Cyan
                } else if key_focused {
                    Color::White
                } else {
                    Color::DarkGray
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

        // "get your key" hint — shown whenever key isn't set yet
        if !wizard.has_existing_image_key() {
            lines.push(Line::from(vec![
                Span::styled("  Get a free key at ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "aistudio.google.com",
                    Style::default()
                        .fg(BRAND_BLUE)
                        .add_modifier(Modifier::UNDERLINED),
                ),
                Span::styled(
                    "  →  Google AI Studio",
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
    }
}

fn render_voice_setup(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    super::onboarding::voice::render(lines, wizard);
}

fn render_daemon(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    let platform = if cfg!(target_os = "linux") {
        "systemd user unit"
    } else if cfg!(target_os = "macos") {
        "LaunchAgent"
    } else {
        "background service"
    };

    lines.push(Line::from(Span::styled(
        format!("  Install as {} ?", platform),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));

    let yes_selected = wizard.install_daemon;
    lines.push(Line::from(vec![
        Span::styled(
            if yes_selected { " > " } else { "   " },
            Style::default().fg(ACCENT_GOLD),
        ),
        Span::styled(
            if yes_selected { "(*)" } else { "( )" },
            Style::default().fg(if yes_selected {
                BRAND_GOLD
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            " Yes, install daemon",
            Style::default().fg(if yes_selected {
                Color::White
            } else {
                Color::DarkGray
            }),
        ),
    ]));

    lines.push(Line::from(vec![
        Span::styled(
            if !yes_selected { " > " } else { "   " },
            Style::default().fg(ACCENT_GOLD),
        ),
        Span::styled(
            if !yes_selected { "(*)" } else { "( )" },
            Style::default().fg(if !yes_selected {
                BRAND_GOLD
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            " Skip for now",
            Style::default().fg(if !yes_selected {
                Color::White
            } else {
                Color::DarkGray
            }),
        ),
    ]));
}

fn render_health_check(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    for (name, status) in &wizard.health_results {
        let (icon, color) = match status {
            HealthStatus::Pending => ("...", Color::DarkGray),
            HealthStatus::Running => ("...", ACCENT_GOLD),
            HealthStatus::Pass => ("OK", Color::Cyan),
            HealthStatus::Fail(_) => ("FAIL", Color::Red),
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!("  [{:<4}] ", icon),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(name.clone(), Style::default().fg(Color::White)),
        ]));

        if let HealthStatus::Fail(reason) = status {
            lines.push(Line::from(Span::styled(
                format!("          {}", reason),
                Style::default().fg(Color::Red),
            )));
        }
    }

    lines.push(Line::from(""));

    if wizard.health_complete {
        if wizard.all_health_passed() {
            lines.push(Line::from(Span::styled(
                "  All checks passed!".to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
            if !wizard.quick_jump {
                lines.push(Line::from(Span::styled(
                    "  Press Enter to finish setup".to_string(),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        } else {
            lines.push(Line::from(Span::styled(
                "  Some checks failed.".to_string(),
                Style::default().fg(Color::Red),
            )));
            if !wizard.quick_jump {
                lines.push(Line::from(vec![
                    Span::styled(
                        "  [R] ",
                        Style::default().fg(BRAND_BLUE).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("Re-run  ", Style::default().fg(Color::White)),
                    Span::styled(
                        "[Esc] ",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("Go back and fix", Style::default().fg(Color::White)),
                ]));
            }
        }
    }
}

fn render_brain_setup(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    // Show generating state
    if wizard.brain_generating {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Cooking up your brain files...".to_string(),
            Style::default()
                .fg(ACCENT_GOLD)
                .add_modifier(Modifier::BOLD | Modifier::ITALIC),
        )));
        lines.push(Line::from(Span::styled(
            "  Your agent is getting to know you".to_string(),
            Style::default().fg(Color::DarkGray),
        )));
        return;
    }

    // Show success state
    if wizard.brain_generated {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Brain files locked in!".to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "  Your agent knows the deal now".to_string(),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Press Enter to finish setup".to_string(),
            Style::default().fg(Color::DarkGray),
        )));
        return;
    }

    // Show error state (with fallback notice)
    if let Some(ref err) = wizard.brain_error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  {} — rolling with defaults", err),
            Style::default().fg(Color::Rgb(215, 100, 20)),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Press Enter to continue".to_string(),
            Style::default().fg(Color::DarkGray),
        )));
        return;
    }

    // "About You" text area
    let me_focused = wizard.brain_field == BrainField::AboutMe;
    lines.push(Line::from(Span::styled(
        "  About You:".to_string(),
        Style::default()
            .fg(if me_focused {
                BRAND_BLUE
            } else {
                Color::DarkGray
            })
            .add_modifier(Modifier::BOLD),
    )));

    let me_display = if wizard.about_me.is_empty() && !me_focused {
        "  name, role, links, projects, whatever you got".to_string()
    } else {
        let cursor = if me_focused { "█" } else { "" };
        format!("  {}{}", wizard.about_me, cursor)
    };
    let me_style = if wizard.about_me.is_empty() && !me_focused {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC)
    } else {
        Style::default().fg(if me_focused {
            Color::White
        } else {
            Color::DarkGray
        })
    };
    // Wrap long text into multiple lines
    for chunk in wrap_text(&me_display, 54) {
        lines.push(Line::from(Span::styled(chunk, me_style)));
    }

    lines.push(Line::from(""));

    // "Your OpenCrabs" text area
    let agent_focused = wizard.brain_field == BrainField::AboutAgent;
    lines.push(Line::from(Span::styled(
        "  Your OpenCrabs:".to_string(),
        Style::default()
            .fg(if agent_focused {
                BRAND_BLUE
            } else {
                Color::DarkGray
            })
            .add_modifier(Modifier::BOLD),
    )));

    let agent_display = if wizard.about_opencrabs.is_empty() && !agent_focused {
        "  personality, vibe, how I should talk to you".to_string()
    } else {
        let cursor = if agent_focused { "█" } else { "" };
        format!("  {}{}", wizard.about_opencrabs, cursor)
    };
    let agent_style = if wizard.about_opencrabs.is_empty() && !agent_focused {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC)
    } else {
        Style::default().fg(if agent_focused {
            Color::White
        } else {
            Color::DarkGray
        })
    };
    for chunk in wrap_text(&agent_display, 54) {
        lines.push(Line::from(Span::styled(chunk, agent_style)));
    }

    lines.push(Line::from(""));
    let italic_style = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::ITALIC);
    for chunk in wrap_text("  The more you drop the better I cover your ass", 54) {
        lines.push(Line::from(Span::styled(chunk, italic_style)));
    }

    // Show loaded hint if brain files exist
    if !wizard.original_about_me.is_empty() || !wizard.original_about_opencrabs.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Loaded from existing brain files".to_string(),
            Style::default().fg(ACCENT_GOLD),
        )));
    }
    let hint_style = Style::default().fg(Color::DarkGray);
    for chunk in wrap_text("  Esc to skip · Tab to switch · Enter to generate", 54) {
        lines.push(Line::from(Span::styled(chunk, hint_style)));
    }
}

/// Wrap a string into chunks of max_width display columns
fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthStr;
    if text.width() <= max_width {
        return vec![text.to_string()];
    }
    let mut result = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        if remaining.width() <= max_width {
            result.push(remaining.to_string());
            break;
        }
        // Find byte index at display width limit
        let byte_limit = super::render::char_boundary_at_width(remaining, max_width);
        // Try to break at a space
        let break_at = remaining[..byte_limit].rfind(' ').unwrap_or(byte_limit);
        let break_at = if break_at == 0 {
            byte_limit.max(remaining.ceil_char_boundary(1))
        } else {
            break_at
        };
        result.push(remaining[..break_at].to_string());
        remaining = remaining[break_at..].trim_start();
    }
    result
}

fn render_complete(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    lines.push(Line::from(""));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Setup complete!".to_string(),
        Style::default().fg(BRAND_GOLD).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    // Summary
    let provider = &PROVIDERS[wizard.ps.selected_provider.min(PROVIDERS.len() - 1)];
    let provider_label = if wizard.ps.selected_provider >= CUSTOM_PROVIDER_IDX
        && !wizard.ps.custom_name.is_empty()
    {
        wizard.ps.custom_name.clone()
    } else {
        provider.name.to_string()
    };
    lines.push(Line::from(vec![
        Span::styled("  Provider: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            provider_label,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    if wizard.ps.is_custom() {
        lines.push(Line::from(vec![
            Span::styled("  Base URL: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                wizard.ps.base_url.clone(),
                Style::default().fg(Color::White),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Model:    ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                wizard.ps.custom_model.clone(),
                Style::default().fg(Color::White),
            ),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("  Model:    ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                wizard.ps.selected_model_name().to_string(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    lines.push(Line::from(vec![
        Span::styled("  Workspace:", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!(" {}", wizard.workspace_path),
            Style::default().fg(Color::White),
        ),
    ]));

    lines.push(Line::from(""));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Entering OpenCrabs...".to_string(),
        Style::default()
            .fg(ACCENT_GOLD)
            .add_modifier(Modifier::BOLD | Modifier::ITALIC),
    )));
}

fn render_trello_setup(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) -> usize {
    let base = lines.len();
    // Help text
    lines.push(Line::from(Span::styled(
        "  1. Go to trello.com/power-ups/admin > Create Power-Up",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(Span::styled(
        "  2. Click 'API Key' tab > copy your API Key",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(Span::styled(
        "  3. Click 'Token' link > authorize > copy Token",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    // API Key input (masked)
    let ak_focused = wizard.trello_field == TrelloField::ApiKey;
    let (masked_ak, ak_hint) = if wizard.has_existing_trello_api_key() {
        (
            "**************************".to_string(),
            " (already configured)".to_string(),
        )
    } else if wizard.trello_api_key_input.is_empty() {
        ("trello-api-key".to_string(), String::new())
    } else {
        (
            "*".repeat(wizard.trello_api_key_input.len().min(30)),
            String::new(),
        )
    };
    let cursor_ak = if ak_focused && !wizard.has_existing_trello_api_key() {
        "\u{2588}"
    } else {
        ""
    };

    lines.push(Line::from(vec![
        Span::styled(
            "  API Key: ",
            Style::default().fg(if ak_focused {
                BRAND_BLUE
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            format!("{}{}", masked_ak, cursor_ak),
            Style::default().fg(if wizard.has_existing_trello_api_key() {
                Color::Cyan
            } else if ak_focused {
                Color::White
            } else {
                Color::DarkGray
            }),
        ),
    ]));

    if !ak_hint.is_empty() && ak_focused {
        lines.push(Line::from(Span::styled(
            format!("  {}", ak_hint.trim()),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    // API Token input (masked)
    let at_focused = wizard.trello_field == TrelloField::ApiToken;
    let (masked_at, at_hint) = if wizard.has_existing_trello_api_token() {
        (
            "**************************".to_string(),
            " (already configured)".to_string(),
        )
    } else if wizard.trello_api_token_input.is_empty() {
        ("trello-api-token".to_string(), String::new())
    } else {
        (
            "*".repeat(wizard.trello_api_token_input.len().min(30)),
            String::new(),
        )
    };
    let cursor_at = if at_focused && !wizard.has_existing_trello_api_token() {
        "\u{2588}"
    } else {
        ""
    };

    lines.push(Line::from(vec![
        Span::styled(
            "  API Token: ",
            Style::default().fg(if at_focused {
                BRAND_BLUE
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            format!("{}{}", masked_at, cursor_at),
            Style::default().fg(if wizard.has_existing_trello_api_token() {
                Color::Cyan
            } else if at_focused {
                Color::White
            } else {
                Color::DarkGray
            }),
        ),
    ]));

    if !at_hint.is_empty() && at_focused {
        lines.push(Line::from(Span::styled(
            format!("  {}", at_hint.trim()),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    // Board(s) input — comma-separated IDs or names (visible)
    let bd_focused = wizard.trello_field == TrelloField::BoardId;
    let (bd_display, bd_hint) = if wizard.has_existing_trello_board_id() {
        (
            "**********".to_string(),
            " (already configured)".to_string(),
        )
    } else if wizard.trello_board_id_input.is_empty() {
        (
            "board-name, id1, workspace-board (comma-separated)".to_string(),
            String::new(),
        )
    } else {
        (wizard.trello_board_id_input.clone(), String::new())
    };
    let bd_cursor = if bd_focused && !wizard.has_existing_trello_board_id() {
        "\u{2588}"
    } else {
        ""
    };

    lines.push(Line::from(vec![
        Span::styled(
            "  Board(s): ",
            Style::default().fg(if bd_focused {
                BRAND_BLUE
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            format!("{}{}", bd_display, bd_cursor),
            Style::default().fg(if wizard.has_existing_trello_board_id() {
                Color::Cyan
            } else if bd_focused {
                Color::White
            } else {
                Color::DarkGray
            }),
        ),
    ]));

    if !bd_hint.is_empty() && bd_focused {
        lines.push(Line::from(Span::styled(
            format!("  {}", bd_hint.trim()),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    // Allowed Users input (member IDs, optional)
    let au_focused = wizard.trello_field == TrelloField::AllowedUsers;
    let (au_display, au_hint) = if wizard.has_existing_trello_allowed_users() {
        (
            "**********".to_string(),
            " (already configured)".to_string(),
        )
    } else if wizard.trello_allowed_users_input.is_empty() {
        (
            "memberid1,memberid2 (optional — empty = reply to all)".to_string(),
            String::new(),
        )
    } else {
        (wizard.trello_allowed_users_input.clone(), String::new())
    };
    let au_cursor = if au_focused && !wizard.has_existing_trello_allowed_users() {
        "\u{2588}"
    } else {
        ""
    };

    lines.push(Line::from(vec![
        Span::styled(
            "  Allowed Users: ",
            Style::default().fg(if au_focused {
                BRAND_BLUE
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            format!("{}{}", au_display, au_cursor),
            Style::default().fg(if wizard.has_existing_trello_allowed_users() {
                Color::Cyan
            } else if au_focused {
                Color::White
            } else {
                Color::DarkGray
            }),
        ),
    ]));

    if !au_hint.is_empty() && au_focused {
        lines.push(Line::from(Span::styled(
            format!("  {}", au_hint.trim()),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    // Test status
    render_channel_test_status(lines, wizard);

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Tab/Shift+Tab: nav fields | \u{2190}\u{2192}: cursor | Ctrl+\u{232b}: clear | Enter: confirm",
        Style::default().fg(Color::DarkGray),
    )));
    let offset = match wizard.trello_field {
        TrelloField::ApiKey => 4,
        TrelloField::ApiToken => 6,
        TrelloField::BoardId => 8,
        TrelloField::AllowedUsers => lines.len().saturating_sub(base).saturating_sub(4),
    };
    base + offset
}
