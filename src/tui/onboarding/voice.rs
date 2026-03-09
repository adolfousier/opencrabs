//! Voice setup step — STT mode selection, API key input, local model picker,
//! TTS mode selection (API vs Local Piper), voice picker, download.

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
        VoiceField::TtsModeSelect => handle_tts_mode(wizard, event.code),
        VoiceField::TtsLocalVoiceSelect => handle_tts_voice(wizard, event.code),
    }
}

fn handle_stt_mode(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    match key {
        KeyCode::Up | KeyCode::Down => {
            // Cycle: 0=Off, 1=API, 2=Local
            wizard.stt_mode = match key {
                KeyCode::Up => {
                    if wizard.stt_mode == 0 {
                        2
                    } else {
                        wizard.stt_mode - 1
                    }
                }
                _ => (wizard.stt_mode + 1) % 3,
            };
        }
        KeyCode::Tab | KeyCode::Enter => {
            match wizard.stt_mode {
                1 => wizard.voice_field = VoiceField::GroqApiKey,
                2 => {
                    wizard.voice_field = VoiceField::LocalModelSelect;
                    refresh_stt_model_status(wizard);
                }
                _ => wizard.voice_field = VoiceField::TtsModeSelect, // Off → skip to TTS
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
            wizard.voice_field = VoiceField::TtsModeSelect;
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
            refresh_stt_model_status(wizard);
        }
        KeyCode::Down if wizard.selected_local_stt_model + 1 < local_stt_model_count() => {
            wizard.selected_local_stt_model += 1;
            wizard.stt_model_download_error = None;
            refresh_stt_model_status(wizard);
        }
        KeyCode::Enter => {
            if wizard.stt_model_downloaded {
                wizard.voice_field = VoiceField::TtsModeSelect;
            } else if wizard.stt_model_download_progress.is_none() {
                return WizardAction::DownloadWhisperModel;
            }
        }
        KeyCode::Tab => {
            wizard.voice_field = VoiceField::TtsModeSelect;
        }
        KeyCode::BackTab => {
            wizard.voice_field = VoiceField::SttModeSelect;
        }
        _ => {}
    }
    WizardAction::None
}

fn handle_tts_mode(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    match key {
        KeyCode::Up | KeyCode::Down => {
            // Cycle: 0=Off, 1=API, 2=Local
            wizard.tts_mode = match key {
                KeyCode::Up => {
                    if wizard.tts_mode == 0 {
                        2
                    } else {
                        wizard.tts_mode - 1
                    }
                }
                _ => (wizard.tts_mode + 1) % 3,
            };
            wizard.tts_enabled = wizard.tts_mode != 0;
        }
        KeyCode::Tab | KeyCode::Enter => {
            if wizard.tts_mode == 2 {
                // Local selected — go to voice picker
                wizard.voice_field = VoiceField::TtsLocalVoiceSelect;
                refresh_tts_voice_status(wizard);
            } else {
                wizard.next_step();
            }
        }
        KeyCode::BackTab => {
            wizard.voice_field = match wizard.stt_mode {
                1 => VoiceField::GroqApiKey,
                2 => VoiceField::LocalModelSelect,
                _ => VoiceField::SttModeSelect, // Off — back to STT selector
            };
        }
        _ => {}
    }
    WizardAction::None
}

fn handle_tts_voice(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    match key {
        KeyCode::Up if wizard.selected_tts_voice > 0 => {
            wizard.selected_tts_voice -= 1;
            wizard.tts_voice_download_error = None;
            wizard.tts_voice_download_progress = None;
            refresh_tts_voice_status(wizard);
        }
        KeyCode::Down if wizard.selected_tts_voice + 1 < tts_voice_count() => {
            wizard.selected_tts_voice += 1;
            wizard.tts_voice_download_error = None;
            wizard.tts_voice_download_progress = None;
            refresh_tts_voice_status(wizard);
        }
        KeyCode::Enter => {
            if wizard.tts_voice_download_progress.is_some() {
                // Download in progress — do nothing
            } else {
                // Always download the selected voice (delete old, download new, play preview)
                return WizardAction::DownloadPiperVoice;
            }
        }
        KeyCode::Tab => {
            wizard.next_step();
        }
        KeyCode::BackTab => {
            wizard.voice_field = VoiceField::TtsModeSelect;
        }
        _ => {}
    }
    WizardAction::None
}

/// Refresh whether the currently selected local STT model is downloaded.
fn refresh_stt_model_status(wizard: &mut OnboardingWizard) {
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

/// Number of local STT model presets available.
fn local_stt_model_count() -> usize {
    #[cfg(feature = "local-stt")]
    {
        crate::channels::voice::local_whisper::LOCAL_MODEL_PRESETS.len()
    }
    #[cfg(not(feature = "local-stt"))]
    {
        0
    }
}

/// Refresh whether the currently selected Piper voice is downloaded.
fn refresh_tts_voice_status(wizard: &mut OnboardingWizard) {
    #[cfg(feature = "local-tts")]
    {
        use crate::channels::voice::local_tts::{PIPER_VOICES, piper_voice_exists};
        if wizard.selected_tts_voice < PIPER_VOICES.len() {
            wizard.tts_voice_downloaded =
                piper_voice_exists(PIPER_VOICES[wizard.selected_tts_voice].id);
        }
    }
    #[cfg(not(feature = "local-tts"))]
    {
        let _ = wizard;
    }
}

/// Number of Piper voice presets available.
fn tts_voice_count() -> usize {
    #[cfg(feature = "local-tts")]
    {
        crate::channels::voice::local_tts::PIPER_VOICES.len()
    }
    #[cfg(not(feature = "local-tts"))]
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

    match wizard.stt_mode {
        1 => render_api_fields(lines, wizard),
        2 => render_local_stt_fields(lines, wizard),
        _ => {} // Off — no sub-fields
    }

    lines.push(Line::from(""));
    render_tts_mode_selector(lines, wizard);

    if wizard.tts_mode == 2 {
        lines.push(Line::from(""));
        render_local_tts_fields(lines, wizard);
    }

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

    render_radio(lines, focused, wizard.stt_mode == 0, "Off");
    render_radio(lines, focused, wizard.stt_mode == 1, "API (Groq Whisper)");
    render_radio(
        lines,
        focused,
        wizard.stt_mode == 2,
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
fn render_local_stt_fields(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
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

fn render_tts_mode_selector(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    let focused = wizard.voice_field == VoiceField::TtsModeSelect;

    lines.push(Line::from(Span::styled(
        "  Text-to-Speech",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        "  Reply with voice notes on channels",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    render_radio(lines, focused, wizard.tts_mode == 0, "Off");
    render_radio(
        lines,
        focused,
        wizard.tts_mode == 1,
        "API (OpenAI TTS \u{2014} uses OpenAI key)",
    );
    render_radio(
        lines,
        focused,
        wizard.tts_mode == 2,
        "Local (Piper \u{2014} runs on device, free)",
    );
}

#[allow(unused_variables)]
fn render_local_tts_fields(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    let focused = wizard.voice_field == VoiceField::TtsLocalVoiceSelect;

    lines.push(Line::from(Span::styled(
        "  Select voice:",
        Style::default().fg(if focused { BRAND_BLUE } else { Color::DarkGray }),
    )));

    #[cfg(feature = "local-tts")]
    {
        use crate::channels::voice::local_tts::{PIPER_VOICES, piper_voice_exists};
        for (i, voice) in PIPER_VOICES.iter().enumerate() {
            let selected = i == wizard.selected_tts_voice;
            let downloaded = piper_voice_exists(voice.id);
            let label = format!(
                "{}{}",
                voice.label,
                if downloaded { " \u{2713}" } else { "" }
            );
            render_radio(lines, focused, selected, &label);
        }
    }

    #[cfg(not(feature = "local-tts"))]
    lines.push(Line::from(Span::styled(
        "  Not available (build with --features local-tts)",
        Style::default().fg(Color::Red),
    )));

    // Download progress / status
    if let Some(progress) = wizard.tts_voice_download_progress {
        render_progress_bar(lines, progress);
    } else if wizard.tts_voice_downloaded {
        lines.push(Line::from(Span::styled(
            "  Voice ready \u{2014} press Enter to continue",
            Style::default().fg(Color::Cyan),
        )));
    } else if let Some(ref err) = wizard.tts_voice_download_error {
        lines.push(Line::from(Span::styled(
            format!("  Download failed: {}", err),
            Style::default().fg(Color::Red),
        )));
    } else if focused {
        lines.push(Line::from(Span::styled(
            "  Press Enter to download voice model",
            Style::default().fg(Color::DarkGray),
        )));
    }
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
