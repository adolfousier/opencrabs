//! Tests for WhatsApp handler: `split_message`, `extract_text`, `has_image`.

use crate::channels::whatsapp::handler::{extract_text, has_image, split_message};
use waproto::whatsapp::Message;

// ── split_message ─────────────────────────────────────────────────────

#[test]
fn split_short_message() {
    let chunks = split_message("hello", 4000);
    assert_eq!(chunks, vec!["hello"]);
}

#[test]
fn split_long_message() {
    let text = "a\n".repeat(3000);
    let chunks = split_message(&text, 4000);
    assert!(chunks.len() >= 2);
    for chunk in &chunks {
        assert!(chunk.len() <= 4000);
    }
    let joined: String = chunks.into_iter().collect();
    assert_eq!(joined, text);
}

// ── extract_text ──────────────────────────────────────────────────────

#[test]
fn extract_text_conversation() {
    let msg = Message {
        conversation: Some("hello".to_string()),
        ..Default::default()
    };
    assert_eq!(extract_text(&msg), Some("hello".to_string()));
}

#[test]
fn extract_text_image_caption() {
    let msg = Message {
        image_message: waproto::whatsapp::message::ImageMessage {
            caption: Some("look at this".to_string()),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };
    assert_eq!(extract_text(&msg), Some("look at this".to_string()));
}

// ── has_image ─────────────────────────────────────────────────────────

#[test]
fn has_image_text_msg() {
    let msg = Message {
        conversation: Some("hi".to_string()),
        ..Default::default()
    };
    assert!(!has_image(&msg));
}

#[test]
fn has_image_img_msg() {
    let msg = Message {
        image_message: waproto::whatsapp::message::ImageMessage::default().into(),
        ..Default::default()
    };
    assert!(has_image(&msg));
}
