//! Voice-note framing for outbound audio (#1486).

use crate::channels::whatsapp::voice_note::{voice_note_mimetype, wants_voice_note};
use serde_json::json;

#[test]
fn audio_defaults_to_a_voice_note() {
    assert!(
        wants_voice_note(&json!({"action": "send_audio", "media_path": "/tmp/a.ogg"})),
        "a bot sending audio means a spoken reply unless told otherwise"
    );
}

#[test]
fn voice_note_false_sends_a_plain_file() {
    assert!(!wants_voice_note(&json!({"voice_note": false})));
}

#[test]
fn voice_note_true_is_honoured() {
    assert!(wants_voice_note(&json!({"voice_note": true})));
}

#[test]
fn a_non_boolean_voice_note_falls_back_to_the_default() {
    // A model that sends the string "false" must not silently get a file
    // attachment; the default stands and the bubble still renders.
    assert!(wants_voice_note(&json!({"voice_note": "false"})));
    assert!(wants_voice_note(&json!({"voice_note": null})));
}

#[test]
fn bare_ogg_gains_the_opus_codec_for_a_voice_note() {
    assert_eq!(
        voice_note_mimetype("audio/ogg", true),
        "audio/ogg; codecs=opus"
    );
}

#[test]
fn bare_ogg_is_untouched_for_a_plain_file_send() {
    assert_eq!(voice_note_mimetype("audio/ogg", false), "audio/ogg");
}

#[test]
fn a_mimetype_that_already_names_its_codec_is_untouched() {
    assert_eq!(
        voice_note_mimetype("audio/ogg; codecs=opus", true),
        "audio/ogg; codecs=opus",
        "rewriting it again would be a no-op at best and a duplicate at worst"
    );
}

#[test]
fn other_containers_are_never_relabelled_as_opus() {
    // Claiming mp3 bytes are Opus would break playback outright.
    for mime in ["audio/mpeg", "audio/mp4", "audio/aac", "audio/wav"] {
        assert_eq!(voice_note_mimetype(mime, true), mime);
    }
}

#[test]
fn mimetype_matching_ignores_case_and_padding() {
    assert_eq!(
        voice_note_mimetype("  AUDIO/OGG  ", true),
        "audio/ogg; codecs=opus"
    );
}
