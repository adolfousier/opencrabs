//! Voice-note framing for outbound audio (#1486).
//!
//! WhatsApp renders audio as a tap-to-play voice bubble only when the message
//! carries `ptt`. Without it the same bytes arrive as an unstyled file
//! attachment, which is what every bot-sent clip looked like. The flag is not
//! quite enough on its own: the client wants Opus-in-Ogg for a voice note, and
//! a bare `audio/ogg` mimetype leaves it guessing.

/// Should this audio go out as a native voice note?
///
/// Defaults to true: a bot sending audio into a chat almost always means a
/// spoken reply, and the caller that genuinely wants a file attachment says
/// so with `voice_note: false`.
pub(crate) fn wants_voice_note(input: &serde_json::Value) -> bool {
    input
        .get("voice_note")
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// Mimetype to stamp on the outgoing `AudioMessage`.
///
/// A voice note is Opus in an Ogg container, and WhatsApp wants the codec
/// spelled out. A bare `audio/ogg` is upgraded; anything else (mp3, m4a, or a
/// mimetype that already names its codec) is passed through untouched, because
/// rewriting it would describe the bytes wrongly.
pub(crate) fn voice_note_mimetype(mime: &str, ptt: bool) -> String {
    if ptt && mime.trim().eq_ignore_ascii_case("audio/ogg") {
        return "audio/ogg; codecs=opus".to_string();
    }
    mime.to_string()
}
