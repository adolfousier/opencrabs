//! Write-contract tests for the Discord TTS voice reply (#1849).
//!
//! These pin what reaches the create request: the voice-message flag, the single
//! `audio/ogg` attachment, and the absence of the fields the platform forbids on
//! a voice message. They deliberately do not claim anything about rendering.
//! Whether a Discord client draws a waveform for an upload that supplies no
//! `waveform` field cannot be settled from source, because serenity 0.12.5 has
//! no way to send that field at all; that is the live-bot check recorded on
//! #1849.

use crate::channels::discord::handler::{plain_attachment_builder, voice_reply_builder};
use serenity::builder::CreateMessage;
use serenity::model::channel::MessageFlags;

fn json(b: &CreateMessage) -> serde_json::Value {
    serde_json::to_value(b).expect("CreateMessage must serialize for the create request")
}

fn raw(b: &CreateMessage) -> String {
    serde_json::to_string(b).expect("CreateMessage must serialize for the create request")
}

/// The documented bit is `1 << 13` (`message.mdx:158`, flag table). If the crate
/// ever disagrees, the code below is sending the wrong number.
#[test]
fn crate_and_docs_agree_on_the_voice_message_bit() {
    assert_eq!(MessageFlags::IS_VOICE_MESSAGE.bits(), 1 << 13);
}

/// The builder must carry exactly the flag the crate defines, whatever its
/// serde encoding turns out to be. Comparing against the crate's own
/// serialization keeps this honest across bitflags representation changes.
#[test]
fn voice_reply_carries_the_voice_message_flag() {
    let built = voice_reply_builder(b"opus payload");
    let expected = serde_json::to_value(MessageFlags::IS_VOICE_MESSAGE).unwrap();
    assert_eq!(json(&built).get("flags"), Some(&expected));
}

/// "Only a single audio attachment is allowed. No content, stickers, etc"
/// (`message.mdx:435-436`). Content is skipped when unset, so its absence in
/// the serialized request is the assertion that holds.
#[test]
fn voice_reply_sends_no_content() {
    let built = voice_reply_builder(b"opus payload");
    assert!(
        json(&built).get("content").is_none(),
        "a voice message must not carry content: {:?}",
        json(&built)
    );
}

/// Same clause covers stickers: `sticker_ids` is not skipped when empty, so an
/// empty array is what the request sends.
#[test]
fn voice_reply_sends_no_stickers_and_no_poll() {
    let j = json(&voice_reply_builder(b"opus payload"));
    assert_eq!(
        j.get("sticker_ids")
            .and_then(|v| v.as_array())
            .map(|a| a.len()),
        Some(0)
    );
    assert!(j.get("poll").is_none());
}

/// One attachment, named `response.ogg`, so serenity's filename-based mime
/// guess yields the `audio/` prefix the contract requires.
#[test]
fn voice_reply_uploads_exactly_one_opus_attachment() {
    let s = raw(&voice_reply_builder(b"opus payload"));
    assert_eq!(
        s.matches("response.ogg").count(),
        1,
        "a voice message allows exactly one attachment: {s}"
    );
}

/// The fallback must stay byte-identical to the pre-#1849 request apart from
/// nothing: same single attachment, and no voice flag at all, so a rejected
/// flagged send degrades to the old plain-file behaviour instead of a second
/// rejection.
#[test]
fn fallback_builder_is_the_pre_flag_shape() {
    let built = plain_attachment_builder(b"opus payload");
    let j = json(&built);
    assert!(
        j.get("flags").is_none(),
        "the fallback must not set the voice flag: {j}"
    );
    assert!(j.get("content").is_none());
    let s = raw(&built);
    assert_eq!(s.matches("response.ogg").count(), 1, "{s}");
}

/// Both builders must agree on the attachment name, or the fallback silently
/// changes the file the user receives.
#[test]
fn both_paths_upload_the_same_filename() {
    let flagged = raw(&voice_reply_builder(b"x"));
    let plain = raw(&plain_attachment_builder(b"x"));
    assert!(flagged.contains("response.ogg") && plain.contains("response.ogg"));
}
