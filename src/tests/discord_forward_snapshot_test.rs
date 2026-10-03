//! #1891: forwarded messages (message_snapshots) must reach the agent turn
//! and the channel history instead of being invisible empty shells.

use crate::channels::discord::handler::forwarded_snapshot_text;
use serenity::model::channel::MessageSnapshot;

/// Minimal valid `MessageSnapshot` fixture. Serenity's struct is
/// `non_exhaustive`, so it can only be built by deserialization.
fn snapshot(content: &str, attachments: serde_json::Value) -> MessageSnapshot {
    serde_json::from_value(serde_json::json!({
        "content": content,
        "timestamp": "2026-10-03T04:43:32.000000+00:00",
        "mentions": [],
        "mention_roles": [],
        "attachments": attachments,
        "embeds": [],
        "type": 0
    }))
    .expect("valid MessageSnapshot fixture")
}

fn attachment(name: &str, mime: &str, url: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "123456789012345678",
        "filename": name,
        "content_type": mime,
        "size": 12345,
        "url": url,
        "proxy_url": url
    })
}

#[test]
fn forwarded_text_surfaces_original_content() {
    let snaps = vec![snapshot("the original words", serde_json::json!([]))];
    let out = forwarded_snapshot_text(&snaps);
    assert!(
        out.contains("[forwarded message]: the original words"),
        "{out}"
    );
}

#[test]
fn forwarded_image_keeps_vision_marker_format() {
    let url = "https://cdn.discordapp.com/attachments/1/2/screenshot.png";
    let snaps = vec![snapshot(
        "look at this",
        serde_json::json!([attachment("screenshot.png", "image/png", url)]),
    )];
    let out = forwarded_snapshot_text(&snaps);
    assert!(
        out.contains("<<IMG:https://cdn.discordapp.com/attachments/1/2/screenshot.png>>"),
        "{out}"
    );
}

#[test]
fn forwarded_file_surfaces_name_and_url() {
    let url = "https://cdn.discordapp.com/attachments/1/2/emoji-pack.zip";
    let snaps = vec![snapshot(
        "",
        serde_json::json!([attachment("emoji-pack.zip", "application/zip", url)]),
    )];
    let out = forwarded_snapshot_text(&snaps);
    assert!(
        out.contains("[forwarded attachment]: emoji-pack.zip"),
        "{out}"
    );
    assert!(out.contains(url), "{out}");
    // No content on the snapshot: no "[forwarded message]" label invented.
    assert!(!out.contains("[forwarded message]"), "{out}");
}

#[test]
fn pure_forward_with_mention_is_not_empty_for_the_guard() {
    // The dispatch guard consumes `content` after this fold; a snapshot-only
    // message (empty msg.content, empty msg.attachments) must yield non-empty
    // text so the guard at handler.rs lets the turn through (#1891 repro:
    // adijr007's forwarded message never dispatched).
    let snaps = vec![snapshot("forwarded payload", serde_json::json!([]))];
    assert!(!forwarded_snapshot_text(&snaps).is_empty());
}

#[test]
fn no_snapshots_adds_nothing() {
    assert_eq!(forwarded_snapshot_text(&[]), "");
}
