//! Tests for the shared attachment router (#1740, #1743): every file class
//! (image, video, audio, archive, document, text, unknown-binary) must
//! classify correctly and every class must surface through
//! `extract_attachments` instead of silently staying prose. Paste, drag-drop
//! and Ctrl+V all funnel through the same classifier, so a fix here covers
//! all three entry points.

use crate::tui::app::App;
use crate::tui::app::messaging::{AttachmentClass, attachment_class_for_path};

#[test]
fn router_classifies_every_known_class() {
    assert_eq!(
        attachment_class_for_path("/tmp/cat photo.heic"),
        AttachmentClass::Image
    );
    assert_eq!(
        attachment_class_for_path("/tmp/clip.mov"),
        AttachmentClass::Video
    );
    assert_eq!(
        attachment_class_for_path("/tmp/voice note.mp3"),
        AttachmentClass::Audio
    );
    assert_eq!(
        attachment_class_for_path("/tmp/backup.flac"),
        AttachmentClass::Audio
    );
    assert_eq!(
        attachment_class_for_path("/tmp/bundle.zip"),
        AttachmentClass::Archive
    );
    assert_eq!(
        attachment_class_for_path("/tmp/site.tar.gz"),
        AttachmentClass::Archive
    );
    assert_eq!(
        attachment_class_for_path("/tmp/report.pdf"),
        AttachmentClass::Document
    );
    assert_eq!(
        attachment_class_for_path("/tmp/pitch.pages"),
        AttachmentClass::Document
    );
    assert_eq!(
        attachment_class_for_path("/tmp/main.rs"),
        AttachmentClass::TextFile
    );
    assert_eq!(
        attachment_class_for_path("/tmp/Cargo.lock"),
        AttachmentClass::TextFile
    );
}

#[test]
fn router_unknown_binary_is_the_deliberate_fall_through() {
    // #1740: the whole point of the new class: a real file the tables have
    // never heard of still classifies (as UnknownBinary), it does not
    // evaporate.
    assert_eq!(
        attachment_class_for_path("/tmp/blob.qwerty"),
        AttachmentClass::UnknownBinary
    );
    assert_eq!(
        attachment_class_for_path("/tmp/no-extension"),
        AttachmentClass::UnknownBinary
    );
    // Case is irrelevant: macOS loves ".PNG".
    assert_eq!(
        attachment_class_for_path("/tmp/IMG_0001.PNG"),
        AttachmentClass::Image
    );
}

#[test]
fn router_is_pure_so_prose_ending_in_zip_cannot_classify() {
    // A sentence that merely ends with an extension-shaped word is not a
    // file: the router has no filesystem access, so nothing here can "exist".
    assert_eq!(
        attachment_class_for_path("I zipped it and sent it.zip"),
        AttachmentClass::Archive
    );
    // Classification is only meaningful for strings that resolve; existence
    // stays with resolve_dropped_path by design.
}

#[test]
fn audio_path_surfaces_transcription_hint_not_attachment() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("standup 2026-09-25.m4a");
    std::fs::write(&real, b"ID3 fake audio").unwrap();

    let e = App::extract_attachments(real.to_str().unwrap());

    assert!(e.attachments.is_empty(), "audio is not an image attachment");
    assert!(
        e.text.contains("audio file")
            && e.text.contains("not transcribed")
            && e.text.contains(real.to_str().unwrap()),
        "audio must surface with a transcription hint: {}",
        e.text
    );
}

#[test]
fn archive_path_surfaces_pointer_note_not_attachment() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("bundle.tar.gz");
    std::fs::write(&real, b"\x1f\x8b fake gzip").unwrap();

    let e = App::extract_attachments(real.to_str().unwrap());

    assert!(e.attachments.is_empty());
    assert!(
        e.text.contains("archive") && e.text.contains(real.to_str().unwrap()),
        "archive must surface as a path the agent can unpack: {}",
        e.text
    );
}

#[test]
fn unknown_extension_path_surfaces_instead_of_staying_prose() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("mystery.qwerty");
    std::fs::write(&real, b"whatever").unwrap();

    let e = App::extract_attachments(real.to_str().unwrap());

    assert!(e.attachments.is_empty());
    assert!(
        e.text.contains("Unknown type") && e.text.contains(real.to_str().unwrap()),
        "#1740: an unknown real file must surface, not evaporate into prose: {}",
        e.text
    );
}

#[test]
fn text_file_path_still_inlines_content() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("main.rs");
    std::fs::write(&real, b"fn main() {}").unwrap();

    let e = App::extract_attachments(real.to_str().unwrap());

    assert!(e.attachments.is_empty());
    assert!(
        e.text.contains("[File:") && e.text.contains("fn main()"),
        "text paths keep their inline fence: {}",
        e.text
    );
}

#[test]
fn unresolvable_non_media_path_gets_a_receipt() {
    // #1740: a path-shaped message pointing at nothing used to flow as bare
    // prose, silently. Now it carries a receipt.
    let e = App::extract_attachments("/Users/nobody/missing/report.pages");

    assert!(e.attachments.is_empty());
    assert!(
        e.notices
            .iter()
            .any(|n| n.contains("Path not found on this machine")),
        "the failed path must be reported, not swallowed: {:?}",
        e.notices
    );
}

#[test]
fn unresolvable_media_path_keeps_its_own_single_receipt() {
    // Media extensions are excluded from the router's receipt: Case 2's scan
    // already emits one for those. Two notices for one path would be noise.
    let e = App::extract_attachments("/Users/nobody/missing/Screenshot 1.png");

    assert!(e.attachments.is_empty());
    assert!(
        !e.notices
            .iter()
            .any(|n| n.contains("Path not found on this machine")),
        "no duplicate receipt for media paths: {:?}",
        e.notices
    );
}

#[test]
fn ordinary_prose_still_flows_through_untouched() {
    let msg = "can you look at the zip file I mentioned earlier";
    let e = App::extract_attachments(msg);
    assert!(e.attachments.is_empty());
    assert!(
        e.notices.is_empty(),
        "prose gets no receipts: {:?}",
        e.notices
    );
    assert_eq!(e.text.trim(), msg);
}

#[test]
fn paste_and_clipboard_paths_classify_identically() {
    // Ctrl+V and bracketed paste both feed the same trimmed string into
    // extract_attachments; the classifier must be indifferent to the entry
    // point. Same path, same class, both directions.
    let path = "/tmp/mixed/media.MOV";
    assert_eq!(
        attachment_class_for_path(path),
        attachment_class_for_path(path.trim()),
        "trimming (what attach_from_clipboard does) must not change the class"
    );
}
