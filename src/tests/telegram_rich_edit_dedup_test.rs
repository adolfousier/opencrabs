//! An edit that would replace a message with itself must not reach Telegram,
//! and must never be logged as a send failure if it does.
//!
//! The rich edit path re-sent identical content on every refresh. Telegram
//! answered `400: message is not modified`, the failure branch logged it at
//! WARN, and one evening of ordinary group use produced dozens of them: each a
//! round trip whose only purpose was to be told nothing changed, spent against
//! a budget that was already backing off elsewhere, and each a line of noise in
//! the log people read when something is genuinely broken (#1443).

use crate::channels::telegram::rich::edit_dedup;

fn edit_body(chat_id: i64, message_id: i32, markdown: &str) -> serde_json::Value {
    serde_json::json!({
        "chat_id": chat_id,
        "message_id": message_id,
        "rich_message": { "markdown": markdown },
    })
}

#[test]
fn an_identical_edit_is_recognised_as_redundant() {
    let body = edit_body(-10099, 7001, "same text unique to this test");
    let (chat, msg, fp) = edit_dedup::fingerprint(&body).expect("an addressed edit");

    assert!(
        !edit_dedup::is_redundant(chat, msg, fp),
        "the first edit of a message has nothing to compare against"
    );

    edit_dedup::remember(chat, msg, fp);

    assert!(
        edit_dedup::is_redundant(chat, msg, fp),
        "re-sending the same content to the same message is the no-op Telegram rejects"
    );
}

#[test]
fn changing_only_the_keyboard_is_still_a_real_edit() {
    let mut body = edit_body(-10099, 7002, "same text unique to this test");
    let (chat, msg, before) = edit_dedup::fingerprint(&body).expect("an addressed edit");
    edit_dedup::remember(chat, msg, before);

    body["reply_markup"] = serde_json::json!({"inline_keyboard": [[{"text": "Approve"}]]});
    let (_, _, after) = edit_dedup::fingerprint(&body).expect("an addressed edit");

    assert_ne!(
        before, after,
        "Telegram compares text AND markup, so hashing the text alone would skip \
         an edit that adds a keyboard"
    );
    assert!(
        !edit_dedup::is_redundant(chat, msg, after),
        "an edit that swaps the keyboard must still be sent"
    );
}

#[test]
fn each_message_is_tracked_separately() {
    let first = edit_body(-10099, 7003, "shared text");
    let second = edit_body(-10099, 7004, "shared text");
    let (chat, msg_a, fp_a) = edit_dedup::fingerprint(&first).expect("addressed");
    let (_, msg_b, fp_b) = edit_dedup::fingerprint(&second).expect("addressed");

    edit_dedup::remember(chat, msg_a, fp_a);

    assert!(
        !edit_dedup::is_redundant(chat, msg_b, fp_b),
        "two messages holding the same text are still two messages; editing one \
         says nothing about the other"
    );
}

#[test]
fn a_body_with_no_message_id_is_not_an_edit() {
    // A send carries no message_id, so there is nothing to deduplicate and the
    // skip must never engage on it.
    let send = serde_json::json!({
        "chat_id": -100,
        "rich_message": { "markdown": "hello" },
    });
    assert!(edit_dedup::fingerprint(&send).is_none());
}

#[test]
fn telegrams_no_op_answer_is_recognised() {
    assert!(edit_dedup::is_not_modified(
        "Bad Request: message is not modified: specified new message content and reply \
         markup are exactly the same as a current content and reply markup of the message"
    ));
    assert!(!edit_dedup::is_not_modified(
        "Bad Request: message to edit not found"
    ));
}
