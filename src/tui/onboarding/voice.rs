//! Voice setup step — STT mode selection, API key input, local model picker, TTS toggle.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::types::{VoiceField, WizardAction};
use super::wizard::OnboardingWizard;

// Brand colors (match onboarding_render.rs)
const BRAND_BLUE: Color = Color::Rgb(60, 130, 246);
const BRAND_GOLD: Color = Color::Rgb(215, 100, 20);
const ACCENT_GOLD: Color = Color::Rgb(215, 100, 20);

// ─── Key handling ───────────────────────────────────────────────────────────

pub fn handle_key(wizard: &mut OnboardingWizard, event: KeyEvent) -> WizardAction {
    match wizard.voice_field {
        VoiceField::SttModeSelect => handle_stt_mode(wizard, event.code),
        VoiceField::GroqApiKey => handle_groq_key(wizard, event.code),
        VoiceField::LocalModelSelect => handle_local_model(wizard, event.code),
        VoiceField::TtsToggle => handle_tts_toggle(wizard, event.code),
    }
}

fn handle_stt_mode(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    match key {
        KeyCode::Up | KeyCode::Down => {
            wizard.stt_mode = 1 - wizard.stt_mode;
        }
        KeyCode::Tab | KeyCode::Enter => {
            if wizard.stt_mode == 0 {
                wizard.voice_field = VoiceField::GroqApiKey;
            } else {
                wizard.voice_field = VoiceField::LocalModelSelect;
                refresh_model_status(wizard);
            }
        }
        _ => {}
    }
    WizardAction::None
}

fn handle_groq_key(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    match key {
        KeyCode::Char(c) => {
            if wizard.has_existing_groq_key() {
                wizard.groq_api_key_input.clear();
            }
            wizard.groq_api_key_input.push(c);
        }
        KeyCode::Backspace => {
            if wizard.has_existing_groq_key() {
                wizard.groq_api_key_input.clear();
            } else {
                wizard.groq_api_key_input.pop();
            }
        }
        KeyCode::Tab | KeyCode::Enter => {
            wizard.voice_field = VoiceField::TtsToggle;
        }
        KeyCode::BackTab => {
            wizard.voice_field = VoiceField::SttModeSelect;
        }
        _ => {}
    }
    WizardAction::None
}

fn handle_local_model(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    match key {
        KeyCode::Up if wizard.selected_local_stt_model > 0 => {
            wizard.selected_local_stt_model -= 1;
            wizard.stt_model_download_error = None;
            refresh_model_status(wizard);
        }
        KeyCode::Down if wizard.selected_local_stt_model + 1 < local_model_count() => {
            wizard.selected_local_stt_model += 1;
            wizard.stt_model_download_error = None;
            refresh_model_status(wizard);
        }
        KeyCode::Enter => {
            if wizard.stt_model_downloaded {
                wizard.voice_field = VoiceField::TtsToggle;
            } else if wizard.stt_model_download_progress.is_none() {
                return WizardAction::DownloadWhisperModel;
            }
        }
        KeyCode::Tab => {
            wizard.voice_field = VoiceField::TtsToggle;
        }
        KeyCode::BackTab => {
            wizard.voice_field = VoiceField::SttModeSelect;
        }
        _ => {}
    }
    WizardAction::None
}

fn handle_tts_toggle(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    match key {
        KeyCode::Char(' ') | KeyCode::Up | KeyCode::Down => {
            wizard.tts_enabled = !wizard.tts_enabled;
        }
        KeyCode::BackTab => {
            wizard.voice_field = if wizard.stt_mode == 0 {
                VoiceField::GroqApiKey
            } else {
                VoiceField::LocalModelSelect
            };
        }
        KeyCode::Tab | KeyCode::Enter => {
            wizard.next_step();
        }
        _ => {}
    }
    WizardAction::None
}

/// Refresh whether the currently selected local model is downloaded.
fn refresh_model_status(wizard: &mut OnboardingWizard) {
    #[cfg(feature = "local-stt")]
    {
        use crate::channels::voice::local_whisper::{LOCAL_MODEL_PRESETS, is_model_downloaded};
        if wizard.selected_local_stt_model < LOCAL_MODEL_PRESETS.len() {
            wizard.stt_model_downloaded =
                is_model_downloaded(&LOCAL_MODEL_PRESETS[wizard.selected_local_stt_model]);
        }
    }
    #[cfg(not(feature = "local-stt"))]
    {
        let _ = wizard;
    }
}

/// Number of local model presets available.
fn local_model_count() -> usize {
    #[cfg(feature = "local-stt")]
    {
        crate::channels::voice::local_whisper::LOCAL_MODEL_PRESETS.len()
    }
    #[cfg(not(feature = "local-stt"))]
    {
        0
    }
}

// ─── Rendering ──────────────────────────────────────────────────────────────

pub fn render(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    // Quick-jump header (deep-link via /onboard:voice)
    if wizard.quick_jump {
        lines.push(Line::from(Span::styled(
            "  Voice Superpowers",
            Style::default().fg(BRAND_GOLD).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "  Talk to me, literally",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
        lines.push(Line::from(""));
    }

    render_stt_mode_selector(lines, wizard);
    lines.push(Line::from(""));

    if wizard.stt_mode == 0 {
        render_api_fields(lines, wizard);
    } else {
        render_local_fields(lines, wizard);
    }

    lines.push(Line::from(""));
    render_tts_toggle(lines, wizard);

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  \u{2191}\u{2193}: select \u{b7} Tab: next \u{b7} Esc: back \u{b7} Enter: continue",
        Style::default().fg(Color::DarkGray),
    )));
}

fn render_stt_mode_selector(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    let focused = wizard.voice_field == VoiceField::SttModeSelect;

    lines.push(Line::from(Span::styled(
        "  Speech-to-Text",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        "  Transcribes voice notes from channels",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    render_radio(lines, focused, wizard.stt_mode == 0, "API (Groq Whisper)");
    render_radio(
        lines,
        focused,
        wizard.stt_mode == 1,
        "Local (whisper.cpp \u{2014} runs on device)",
    );
}

fn render_api_fields(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    let focused = wizard.voice_field == VoiceField::GroqApiKey;
    let (masked, hint) = if wizard.has_existing_groq_key() {
        ("**************************", " (already configured)")
    } else if wizard.groq_api_key_input.is_empty() {
        ("get key from console.groq.com", "")
    } else {
        ("", "") // handled below
    };

    let display = if !wizard.has_existing_groq_key() && !wizard.groq_api_key_input.is_empty() {
        "*".repeat(wizard.groq_api_key_input.len().min(30))
    } else {
        masked.to_string()
    };

    let cursor = if focused && !wizard.has_existing_groq_key() {
        "\u{2588}"
    } else {
        ""
    };

    lines.push(Line::from(vec![
        Span::styled(
            "  Groq Key: ",
            Style::default().fg(if focused { BRAND_BLUE } else { Color::DarkGray }),
        ),
        Span::styled(
            format!("{}{}", display, cursor),
            Style::default().fg(if wizard.has_existing_groq_key() {
                Color::Cyan
            } else if focused {
                Color::White
            } else {
                Color::DarkGray
            }),
        ),
    ]));

    if !hint.is_empty() && focused {
        lines.push(Line::from(Span::styled(
            format!("  {}", hint.trim()),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }
}

#[allow(unused_variables)]
fn render_local_fields(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    let focused = wizard.voice_field == VoiceField::LocalModelSelect;

    lines.push(Line::from(Span::styled(
        "  Select model size:",
        Style::default().fg(if focused { BRAND_BLUE } else { Color::DarkGray }),
    )));

    #[cfg(feature = "local-stt")]
    {
        use crate::channels::voice::local_whisper::{LOCAL_MODEL_PRESETS, is_model_downloaded};
        for (i, preset) in LOCAL_MODEL_PRESETS.iter().enumerate() {
            let selected = i == wizard.selected_local_stt_model;
            let downloaded = is_model_downloaded(preset);
            let label = format!(
                "{} ({}){}",
                preset.label,
                preset.size_label,
                if downloaded { " \u{2713}" } else { "" }
            );
            render_radio(lines, focused, selected, &label);
        }
    }

    #[cfg(not(feature = "local-stt"))]
    lines.push(Line::from(Span::styled(
        "  Not available (build with --features local-stt)",
        Style::default().fg(Color::Red),
    )));

    // Download progress / status
    if let Some(progress) = wizard.stt_model_download_progress {
        render_progress_bar(lines, progress);
    } else if wizard.stt_model_downloaded {
        lines.push(Line::from(Span::styled(
            "  Model ready \u{2014} press Enter to continue",
            Style::default().fg(Color::Cyan),
        )));
    } else if let Some(ref err) = wizard.stt_model_download_error {
        lines.push(Line::from(Span::styled(
            format!("  Download failed: {}", err),
            Style::default().fg(Color::Red),
        )));
    } else if focused {
        lines.push(Line::from(Span::styled(
            "  Press Enter to download",
            Style::default().fg(Color::DarkGray),
        )));
    }
}

fn render_tts_toggle(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    let focused = wizard.voice_field == VoiceField::TtsToggle;

    lines.push(Line::from(Span::styled(
        "  Text-to-Speech (OpenAI TTS)",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        "  Reply with voice notes (uses OpenAI key)",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    lines.push(Line::from(vec![
        Span::styled(
            if focused { " > " } else { "   " },
            Style::default().fg(ACCENT_GOLD),
        ),
        Span::styled(
            if wizard.tts_enabled { "[x]" } else { "[ ]" },
            Style::default().fg(if wizard.tts_enabled {
                BRAND_GOLD
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            " Enable TTS replies (ash voice)",
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
}

// ─── Shared helpers ─────────────────────────────────────────────────────────

fn render_radio(lines: &mut Vec<Line<'static>>, focused: bool, selected: bool, label: &str) {
    lines.push(Line::from(vec![
        Span::styled(
            if focused && selected { " > " } else { "   " },
            Style::default().fg(ACCENT_GOLD),
        ),
        Span::styled(
            if selected { "(*)" } else { "( )" },
            Style::default().fg(if selected {
                BRAND_GOLD
            } else {
                Color::DarkGray
            }),
        ),
        Span::styled(
            format!(" {}", label),
            Style::default()
                .fg(if focused && selected {
                    Color::White
                } else {
                    Color::DarkGray
                })
                .add_modifier(if focused && selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
    ]));
}

fn render_progress_bar(lines: &mut Vec<Line<'static>>, progress: f64) {
    let pct = (progress * 100.0) as u32;
    let bar_width = 20;
    let filled = (progress * bar_width as f64) as usize;
    let empty = bar_width - filled;
    lines.push(Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled("\u{2588}".repeat(filled), Style::default().fg(BRAND_GOLD)),
        Span::styled(
            "\u{2591}".repeat(empty),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(format!(" {}%", pct), Style::default().fg(Color::White)),
    ]));
}
