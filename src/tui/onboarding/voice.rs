//! Voice setup step — STT/TTS provider selection, API key input, local model picker.
//!
//! STT providers: Off, Groq, Local, OpenAI-compatible, Voicebox
//! TTS providers: Off, OpenAI, Local, OpenAI-compatible, Voicebox

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::types::{SttProvider, TtsProvider, VoiceField, WizardAction};
use super::wizard::OnboardingWizard;

// Brand colors (match onboarding_render.rs)
const BRAND_BLUE: Color = Color::Rgb(60, 130, 246);
const ACCENT_GOLD: Color = Color::Rgb(215, 100, 20);

// ─── Key handling ───────────────────────────────────────────────────────────

pub fn handle_key(wizard: &mut OnboardingWizard, event: KeyEvent) -> WizardAction {
    match wizard.voice_field {
        VoiceField::SttModeSelect => handle_stt_mode(wizard, event.code),
        VoiceField::GroqApiKey => handle_groq_key(wizard, event.code),
        VoiceField::LocalModelSelect => handle_local_model(wizard, event.code),
        VoiceField::SttOpenaiCompatSelect => handle_stt_oc_select(wizard, event.code),
        VoiceField::SttOpenaiCompatUrl => handle_text_field(
            wizard,
            event.code,
            |w| &mut w.stt_openai_compat_base_url,
            VoiceField::SttOpenaiCompatModel,
            VoiceField::SttOpenaiCompatSelect,
        ),
        VoiceField::SttOpenaiCompatModel => handle_text_field(
            wizard,
            event.code,
            |w| &mut w.stt_openai_compat_model,
            VoiceField::SttOpenaiCompatKey,
            VoiceField::SttOpenaiCompatUrl,
        ),
        VoiceField::SttOpenaiCompatKey => handle_text_field(
            wizard,
            event.code,
            |w| &mut w.stt_openai_compat_key_input,
            VoiceField::SttVoiceboxSelect,
            VoiceField::SttOpenaiCompatModel,
        ),
        VoiceField::SttVoiceboxSelect => handle_stt_voicebox_select(wizard, event.code),
        VoiceField::SttVoiceboxUrl => handle_text_field(
            wizard,
            event.code,
            |w| &mut w.stt_voicebox_base_url,
            VoiceField::TtsModeSelect,
            VoiceField::SttVoiceboxSelect,
        ),
        VoiceField::TtsModeSelect => handle_tts_mode(wizard, event.code),
        VoiceField::TtsLocalVoiceSelect => handle_tts_voice(wizard, event.code),
        VoiceField::TtsOpenaiCompatSelect => handle_tts_oc_select(wizard, event.code),
        VoiceField::TtsOpenaiCompatUrl => handle_text_field(
            wizard,
            event.code,
            |w| &mut w.tts_openai_compat_base_url,
            VoiceField::TtsOpenaiCompatModel,
            VoiceField::TtsOpenaiCompatSelect,
        ),
        VoiceField::TtsOpenaiCompatModel => handle_text_field(
            wizard,
            event.code,
            |w| &mut w.tts_openai_compat_model,
            VoiceField::TtsOpenaiCompatVoice,
            VoiceField::TtsOpenaiCompatUrl,
        ),
        VoiceField::TtsOpenaiCompatVoice => handle_text_field(
            wizard,
            event.code,
            |w| &mut w.tts_openai_compat_voice,
            VoiceField::TtsOpenaiCompatKey,
            VoiceField::TtsOpenaiCompatModel,
        ),
        VoiceField::TtsOpenaiCompatKey => handle_text_field(
            wizard,
            event.code,
            |w| &mut w.tts_openai_compat_key_input,
            VoiceField::TtsVoiceboxSelect,
            VoiceField::TtsOpenaiCompatVoice,
        ),
        VoiceField::TtsVoiceboxSelect => handle_tts_voicebox_select(wizard, event.code),
        VoiceField::TtsVoiceboxUrl => handle_text_field(
            wizard,
            event.code,
            |w| &mut w.tts_voicebox_base_url,
            VoiceField::TtsVoiceboxProfileId,
            VoiceField::TtsVoiceboxSelect,
        ),
        VoiceField::TtsVoiceboxProfileId => handle_text_field(
            wizard,
            event.code,
            |w| &mut w.tts_voicebox_profile_id,
            VoiceField::TtsVoiceboxEngine,
            VoiceField::TtsVoiceboxUrl,
        ),
        VoiceField::TtsVoiceboxEngine => handle_text_field(
            wizard,
            event.code,
            |w| &mut w.tts_voicebox_engine,
            VoiceField::Continue,
            VoiceField::TtsVoiceboxProfileId,
        ),
        VoiceField::Continue => handle_continue(wizard, event.code),
    }
}

// ─── STT mode ───────────────────────────────────────────────────────────────

fn handle_stt_mode(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    let available = SttProvider::available(crate::channels::voice::local_stt_available());
    match key {
        KeyCode::Up => {
            wizard.stt_provider = wizard.stt_provider.prev(available);
        }
        KeyCode::Down => {
            wizard.stt_provider = wizard.stt_provider.next(available);
        }
        KeyCode::Tab | KeyCode::Enter => {
            advance_from_stt(wizard);
        }
        KeyCode::BackTab => {
            wizard.voice_field = VoiceField::Continue;
        }
        _ => {}
    }
    WizardAction::None
}

fn advance_from_stt(wizard: &mut OnboardingWizard) {
    match wizard.stt_provider {
        SttProvider::Off => wizard.voice_field = VoiceField::TtsModeSelect,
        SttProvider::Groq => wizard.voice_field = VoiceField::GroqApiKey,
        SttProvider::Local => {
            wizard.voice_field = VoiceField::LocalModelSelect;
            refresh_stt_model_status(wizard);
        }
        SttProvider::OpenAiCompatible => wizard.voice_field = VoiceField::SttOpenaiCompatUrl,
        SttProvider::Voicebox => wizard.voice_field = VoiceField::SttVoiceboxUrl,
    }
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
        KeyCode::Tab | KeyCode::Enter => wizard.voice_field = VoiceField::TtsModeSelect,
        KeyCode::BackTab => wizard.voice_field = VoiceField::SttModeSelect,
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
        KeyCode::Tab => wizard.voice_field = VoiceField::TtsModeSelect,
        KeyCode::BackTab => wizard.voice_field = VoiceField::SttModeSelect,
        _ => {}
    }
    WizardAction::None
}

// ─── OpenAI-compatible STT ──────────────────────────────────────────────────

fn handle_stt_oc_select(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    match key {
        KeyCode::Tab | KeyCode::Enter => wizard.voice_field = VoiceField::SttOpenaiCompatUrl,
        KeyCode::BackTab => wizard.voice_field = VoiceField::SttModeSelect,
        _ => {}
    }
    WizardAction::None
}

// ─── Voicebox STT ───────────────────────────────────────────────────────────

fn handle_stt_voicebox_select(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    match key {
        KeyCode::Tab | KeyCode::Enter => wizard.voice_field = VoiceField::SttVoiceboxUrl,
        KeyCode::BackTab => wizard.voice_field = VoiceField::SttOpenaiCompatSelect,
        _ => {}
    }
    WizardAction::None
}

// ─── TTS mode ───────────────────────────────────────────────────────────────

fn handle_tts_mode(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    let available = TtsProvider::available(crate::channels::voice::local_tts_available());
    match key {
        KeyCode::Up => {
            wizard.tts_provider = wizard.tts_provider.prev(available);
        }
        KeyCode::Down => {
            wizard.tts_provider = wizard.tts_provider.next(available);
        }
        KeyCode::Tab | KeyCode::Enter => {
            advance_from_tts(wizard);
        }
        KeyCode::BackTab => {
            // Go to the last field of whatever STT provider is selected
            match wizard.stt_provider {
                SttProvider::Voicebox => wizard.voice_field = VoiceField::SttVoiceboxUrl,
                SttProvider::OpenAiCompatible => {
                    wizard.voice_field = VoiceField::SttOpenaiCompatKey
                }
                SttProvider::Local => wizard.voice_field = VoiceField::LocalModelSelect,
                SttProvider::Groq => wizard.voice_field = VoiceField::GroqApiKey,
                SttProvider::Off => wizard.voice_field = VoiceField::SttModeSelect,
            }
        }
        _ => {}
    }
    WizardAction::None
}

fn advance_from_tts(wizard: &mut OnboardingWizard) {
    wizard.tts_enabled = wizard.tts_provider != TtsProvider::Off;
    match wizard.tts_provider {
        TtsProvider::Off => wizard.voice_field = VoiceField::Continue,
        TtsProvider::OpenAi => wizard.voice_field = VoiceField::Continue,
        TtsProvider::Local => {
            wizard.voice_field = VoiceField::TtsLocalVoiceSelect;
            refresh_tts_voice_status(wizard);
        }
        TtsProvider::OpenAiCompatible => wizard.voice_field = VoiceField::TtsOpenaiCompatUrl,
        TtsProvider::Voicebox => wizard.voice_field = VoiceField::TtsVoiceboxUrl,
    }
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
                // downloading — do nothing
            } else if !wizard.tts_voice_downloaded {
                return WizardAction::DownloadPiperVoice;
            } else {
                wizard.voice_field = VoiceField::Continue;
            }
        }
        KeyCode::Tab => wizard.voice_field = VoiceField::Continue,
        KeyCode::BackTab => wizard.voice_field = VoiceField::TtsModeSelect,
        _ => {}
    }
    WizardAction::None
}

// ─── OpenAI-compatible TTS ──────────────────────────────────────────────────

fn handle_tts_oc_select(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    match key {
        KeyCode::Tab | KeyCode::Enter => wizard.voice_field = VoiceField::TtsOpenaiCompatUrl,
        KeyCode::BackTab => wizard.voice_field = VoiceField::TtsModeSelect,
        _ => {}
    }
    WizardAction::None
}

// ─── Voicebox TTS ───────────────────────────────────────────────────────────

fn handle_tts_voicebox_select(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    match key {
        KeyCode::Tab | KeyCode::Enter => wizard.voice_field = VoiceField::TtsVoiceboxUrl,
        KeyCode::BackTab => wizard.voice_field = VoiceField::TtsOpenaiCompatSelect,
        _ => {}
    }
    WizardAction::None
}

// ─── Continue ───────────────────────────────────────────────────────────────

fn handle_continue(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    match key {
        KeyCode::Enter => wizard.next_step(),
        KeyCode::Tab => wizard.voice_field = VoiceField::SttModeSelect,
        KeyCode::BackTab => {
            // Go back to the last field of whatever TTS provider is selected
            wizard.voice_field = match wizard.tts_provider {
                TtsProvider::Voicebox => VoiceField::TtsVoiceboxEngine,
                TtsProvider::OpenAiCompatible => VoiceField::TtsOpenaiCompatKey,
                TtsProvider::Local => VoiceField::TtsLocalVoiceSelect,
                TtsProvider::OpenAi | TtsProvider::Off => VoiceField::TtsModeSelect,
            };
        }
        _ => {}
    }
    WizardAction::None
}

// ─── Generic text field handler ─────────────────────────────────────────────

fn handle_text_field<F>(
    wizard: &mut OnboardingWizard,
    key: KeyCode,
    get_field: F,
    forward: VoiceField,
    back: VoiceField,
) -> WizardAction
where
    F: FnOnce(&mut OnboardingWizard) -> &mut String,
{
    match key {
        KeyCode::Char(c) => get_field(wizard).push(c),
        KeyCode::Backspace => {
            get_field(wizard).pop();
        }
        KeyCode::Tab | KeyCode::Enter => {
            wizard.voice_field = forward;
        }
        KeyCode::BackTab => {
            wizard.voice_field = back;
        }
        _ => {}
    }
    WizardAction::None
}

// ─── Status helpers ─────────────────────────────────────────────────────────

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

// ─── Rendering ─────────────────────────────────────────────────────────────

pub fn render(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    if wizard.quick_jump {
        lines.push(Line::from(Span::styled(
            "  Voice Superpowers",
            Style::default()
                .fg(ACCENT_GOLD)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "  Talk to me, literally",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
        lines.push(Line::from(""));
    }

    // ── STT section ──
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
    render_stt_mode_selector(lines, wizard);
    lines.push(Line::from(""));

    // STT fields for the selected provider
    match wizard.stt_provider {
        SttProvider::Off => {}
        SttProvider::Groq => render_groq_fields(lines, wizard),
        SttProvider::Local => render_local_stt_fields(lines, wizard),
        SttProvider::OpenAiCompatible => render_stt_openai_compat_fields(lines, wizard),
        SttProvider::Voicebox => render_stt_voicebox_fields(lines, wizard),
    }

    lines.push(Line::from(""));

    // ── TTS section ──
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
    render_tts_mode_selector(lines, wizard);
    lines.push(Line::from(""));

    // TTS fields for the selected provider
    match wizard.tts_provider {
        TtsProvider::Off => {}
        TtsProvider::OpenAi => {}
        TtsProvider::Local => render_local_tts_fields(lines, wizard),
        TtsProvider::OpenAiCompatible => render_tts_openai_compat_fields(lines, wizard),
        TtsProvider::Voicebox => render_tts_voicebox_fields(lines, wizard),
    }

    // Continue button
    lines.push(Line::from(""));
    let continue_focused = wizard.voice_field == VoiceField::Continue;
    if continue_focused {
        lines.push(Line::from(vec![
            Span::styled(
                "  > ",
                Style::default()
                    .fg(ACCENT_GOLD)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Continue",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            "    Continue",
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  \u{2191}\u{2193}: select \u{b7} Tab: next field \u{b7} Esc: back \u{b7} Enter: confirm",
        Style::default().fg(Color::DarkGray),
    )));
}

// ─── STT rendering ──────────────────────────────────────────────────────────

fn render_stt_mode_selector(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    let focused = wizard.voice_field == VoiceField::SttModeSelect;
    for provider in SttProvider::available(crate::channels::voice::local_stt_available()) {
        render_radio(
            lines,
            focused,
            wizard.stt_provider == *provider,
            provider.label(),
        );
    }
}

fn render_groq_fields(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    let focused = wizard.voice_field == VoiceField::GroqApiKey;
    let (masked, hint) = if wizard.has_existing_groq_key() {
        ("**************************", " (already configured)")
    } else if wizard.groq_api_key_input.is_empty() {
        ("get key from console.groq.com", "")
    } else {
        ("", "")
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

fn render_stt_openai_compat_fields(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    let base_focused = wizard.voice_field == VoiceField::SttOpenaiCompatUrl;
    render_text_field(
        lines,
        "  Base URL: ",
        &wizard.stt_openai_compat_base_url,
        "https://api.openai.com/v1",
        base_focused,
    );

    let model_focused = wizard.voice_field == VoiceField::SttOpenaiCompatModel;
    render_text_field(
        lines,
        "  Model: ",
        &wizard.stt_openai_compat_model,
        "whisper-1",
        model_focused,
    );

    let key_focused = wizard.voice_field == VoiceField::SttOpenaiCompatKey;
    let has_key = !wizard.stt_openai_compat_key_input.is_empty()
        && wizard.stt_openai_compat_key_input != super::types::EXISTING_KEY_SENTINEL;
    render_text_field(
        lines,
        "  API Key: ",
        &mask_if_not_empty(&wizard.stt_openai_compat_key_input),
        "optional",
        key_focused,
    );
    if key_focused && !has_key {
        lines.push(Line::from(Span::styled(
            "    (API key is optional for this provider)",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }
}

fn render_stt_voicebox_fields(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    let url_focused = wizard.voice_field == VoiceField::SttVoiceboxUrl;
    render_text_field(
        lines,
        "  Base URL: ",
        &wizard.stt_voicebox_base_url,
        "http://localhost:8000",
        url_focused,
    );
}

// ─── TTS rendering ──────────────────────────────────────────────────────────

fn render_tts_mode_selector(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    let focused = wizard.voice_field == VoiceField::TtsModeSelect;
    for provider in TtsProvider::available(crate::channels::voice::local_tts_available()) {
        render_radio(
            lines,
            focused,
            wizard.tts_provider == *provider,
            provider.label(),
        );
    }
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

fn render_tts_openai_compat_fields(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    let base_focused = wizard.voice_field == VoiceField::TtsOpenaiCompatUrl;
    render_text_field(
        lines,
        "  Base URL: ",
        &wizard.tts_openai_compat_base_url,
        "https://api.openai.com/v1",
        base_focused,
    );

    let model_focused = wizard.voice_field == VoiceField::TtsOpenaiCompatModel;
    render_text_field(
        lines,
        "  Model: ",
        &wizard.tts_openai_compat_model,
        "tts-1",
        model_focused,
    );

    let voice_focused = wizard.voice_field == VoiceField::TtsOpenaiCompatVoice;
    render_text_field(
        lines,
        "  Voice: ",
        &wizard.tts_openai_compat_voice,
        "alloy",
        voice_focused,
    );

    let key_focused = wizard.voice_field == VoiceField::TtsOpenaiCompatKey;
    let has_key = !wizard.tts_openai_compat_key_input.is_empty()
        && wizard.tts_openai_compat_key_input != super::types::EXISTING_KEY_SENTINEL;
    render_text_field(
        lines,
        "  API Key: ",
        &mask_if_not_empty(&wizard.tts_openai_compat_key_input),
        "optional",
        key_focused,
    );
    if key_focused && !has_key {
        lines.push(Line::from(Span::styled(
            "    (API key is optional for this provider)",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }
}

fn render_tts_voicebox_fields(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    let url_focused = wizard.voice_field == VoiceField::TtsVoiceboxUrl;
    render_text_field(
        lines,
        "  Base URL: ",
        &wizard.tts_voicebox_base_url,
        "http://localhost:8000",
        url_focused,
    );

    let profile_focused = wizard.voice_field == VoiceField::TtsVoiceboxProfileId;
    render_text_field(
        lines,
        "  Profile ID: ",
        &wizard.tts_voicebox_profile_id,
        "",
        profile_focused,
    );

    let engine_focused = wizard.voice_field == VoiceField::TtsVoiceboxEngine;
    render_text_field(
        lines,
        "  Engine: ",
        &wizard.tts_voicebox_engine,
        "kokoro, qwen, qwen_custom_voice",
        engine_focused,
    );
}

// ─── Shared rendering helpers ───────────────────────────────────────────────

fn render_radio(lines: &mut Vec<Line<'static>>, focused: bool, selected: bool, label: &str) {
    lines.push(Line::from(vec![
        Span::styled(
            if focused && selected { " > " } else { "   " },
            Style::default().fg(ACCENT_GOLD),
        ),
        Span::styled(
            if selected { "(*)" } else { "( )" },
            Style::default().fg(if selected {
                ACCENT_GOLD
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

fn mask_if_not_empty(s: &str) -> String {
    if s.is_empty() || s == super::types::EXISTING_KEY_SENTINEL {
        String::new()
    } else {
        "*".repeat(s.len().min(20))
    }
}

fn render_text_field(
    lines: &mut Vec<Line<'static>>,
    label: &'static str,
    value: &str,
    placeholder: &'static str,
    focused: bool,
) {
    let display = if value.is_empty() {
        placeholder.to_string()
    } else {
        value.to_string()
    };
    let color = if focused { BRAND_BLUE } else { Color::DarkGray };
    let text_color = if value.is_empty() {
        Color::DarkGray
    } else if focused {
        Color::White
    } else {
        Color::Gray
    };
    let cursor = if focused && !value.is_empty() {
        "\u{2588}"
    } else {
        ""
    };

    lines.push(Line::from(vec![
        Span::styled(label, Style::default().fg(color)),
        Span::styled(
            format!("{}{}", display, cursor),
            Style::default().fg(text_color),
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
        Span::styled("\u{2588}".repeat(filled), Style::default().fg(ACCENT_GOLD)),
        Span::styled(
            "\u{2591}".repeat(empty),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(format!(" {}%", pct), Style::default().fg(Color::White)),
    ]));
}
