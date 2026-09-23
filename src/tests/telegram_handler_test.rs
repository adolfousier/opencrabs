//! Tests for Telegram handler: `split_message`, `markdown_to_telegram_html`, `escape_html`.

use crate::channels::telegram::handler::{
    build_midturn_queued_message, channel_id_hint, escape_html, markdown_to_telegram_html,
    split_message, strip_command_mention_suffix,
};

// ── split_message ─────────────────────────────────────────────────────

#[test]
fn split_short_message() {
    let chunks = split_message("hello", 4096);
    assert_eq!(chunks, vec!["hello"]);
}

#[test]
fn split_long_message() {
    let text = "a\n".repeat(3000);
    let chunks = split_message(&text, 4096);
    assert!(chunks.len() >= 2);
    for chunk in &chunks {
        assert!(chunk.len() <= 4096);
    }
    let joined: String = chunks.into_iter().collect();
    assert_eq!(joined, text);
}

#[test]
fn split_no_newlines() {
    let text = "a".repeat(5000);
    let chunks = split_message(&text, 4096);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].len(), 4096);
    assert_eq!(chunks[1].len(), 904);
}

// ── markdown_to_telegram_html ─────────────────────────────────────────

#[test]
fn markdown_bold() {
    let html = markdown_to_telegram_html("**hello**");
    assert!(html.contains("<b>hello</b>"));
}

#[test]
fn markdown_code_block() {
    let md = "```rust\nfn main() {}\n```";
    let html = markdown_to_telegram_html(md);
    assert!(html.contains("<pre><code"));
    assert!(html.contains("fn main()"));
    assert!(html.contains("</code></pre>"));
}

#[test]
fn markdown_inline_code() {
    let html = markdown_to_telegram_html("use `cargo build`");
    assert!(html.contains("<code>cargo build</code>"));
}

// ── escape_html ───────────────────────────────────────────────────────

#[test]
fn escape_html_tags() {
    assert_eq!(
        escape_html("<script>alert('xss')</script>"),
        "&lt;script&gt;alert('xss')&lt;/script&gt;"
    );
}

#[test]
fn escape_html_ampersand() {
    assert_eq!(escape_html("a & b"), "a &amp; b");
}

// ── IMG marker format ─────────────────────────────────────────────────

#[test]
fn img_marker_format() {
    let path = "/tmp/tg_photo_abc.jpg";
    let caption = "What's in this image?";
    let text = format!("<<IMG:{}>> {}", path, caption);
    assert!(text.starts_with("<<IMG:"));
    assert!(text.contains(path));
    assert!(text.contains(caption));
}

// ── build_midturn_queued_message: slash command vs plain follow-up ───
// A slash command that lands mid-turn is a deliberate NEW directive and must
// NOT get the "fold into the current task, do not restart" wrapper a plain
// follow-up gets — that wrapper neutralized /drop_release so the release
// never ran. Slash commands get a directive wrapper and show the command in
// history; plain follow-ups keep the original behavior byte-for-byte.

#[test]
fn slash_command_midturn_is_a_distinct_directive() {
    let body = "# Drop Release\nYou are preparing a new release. Follow every step.";
    let q = build_midturn_queued_message(Some("/drop_release"), body, "ignored display");
    // Names the command and frames it as a NEW directive, not a refinement.
    assert!(q.context_text.contains("/drop_release"));
    assert!(q.context_text.contains("explicit NEW directive"));
    assert!(
        q.context_text
            .contains("carry out the following instructions")
    );
    // The resolved body is carried through so the command can actually run.
    assert!(q.context_text.contains("preparing a new release"));
    // It must NOT carry the follow-up "do not restart" framing.
    assert!(!q.context_text.contains("do not restart from scratch"));
    // History shows the command the user typed, not the whole body.
    assert_eq!(q.display_text, "/drop_release");
}

#[test]
fn plain_followup_midturn_keeps_the_fold_in_wrapper() {
    let q = build_midturn_queued_message(None, "resolved", "also check the logs");
    assert!(q.context_text.contains("factor it into the CURRENT task"));
    assert!(q.context_text.contains("do not restart from scratch"));
    assert!(q.context_text.contains("also check the logs"));
    // A plain follow-up must NOT read as a command invocation.
    assert!(!q.context_text.contains("explicit NEW directive"));
    assert_eq!(q.display_text, "also check the logs");
}

// ── channel_id_hint: chat_id / thread_id in the [Channel: ...] header (#533)
// The agent needs the current chat_id (and forum thread_id) to target this
// conversation for cron reports / cross-surface sends without guessing.

#[test]
fn channel_id_hint_includes_thread_for_forum_topics() {
    assert_eq!(
        channel_id_hint(-1001234567890, Some(12)),
        "chat_id: -1001234567890, thread_id: 12"
    );
}

#[test]
fn channel_id_hint_omits_thread_for_plain_chats() {
    assert_eq!(channel_id_hint(8535704842, None), "chat_id: 8535704842");
}

// ── #1708: the banner's thread id must be the SAME value session routing
// uses — topic_session_id-gated, never the raw per-reply id.

#[test]
fn banner_never_shows_a_bare_reply_chain_thread_id() {
    use crate::channels::telegram::session_resolve::topic_session_id;

    // Production (#1708): a plain reply in a NON-forum group carried
    // thread_id 215/220. The old banner displayed them verbatim, so the
    // model read them as forum topics and bound phantom ones. Gated, the
    // banner degrades to the bare chat id.
    for stray in [215, 220] {
        let shown = channel_id_hint(
            -1004233630800,
            topic_session_id(/* is_topic_message */ false, Some(stray)),
        );
        assert_eq!(
            shown, "chat_id: -1004233630800",
            "#1708: stray reply-chain id {stray} must not reach the banner"
        );
    }

    // A real forum topic still names itself, so cross-surface sends keep
    // working there (#215 behaviour unchanged).
    assert_eq!(
        channel_id_hint(-1001234567890, topic_session_id(true, Some(30045))),
        "chat_id: -1001234567890, thread_id: 30045"
    );
}

#[test]
fn banner_never_names_general() {
    use crate::channels::telegram::session_resolve::{GENERAL_TOPIC_ID, topic_session_id};

    // The issue's literal suggestion (session_topic_for_event output) would
    // normalize a known forum's General to Some(1) and put "thread_id: 1" on
    // the banner — the model would then copy 1 onto the wire and every send
    // would be refused (#1319). The gate keeps General unnamed instead.
    let shown = channel_id_hint(
        -1001234567890,
        topic_session_id(/* is_topic_message */ false, Some(GENERAL_TOPIC_ID)),
    );
    assert_eq!(shown, "chat_id: -1001234567890");
    assert!(
        !shown.contains("thread_id"),
        "#1319: never advertise thread 1"
    );
}

// ── strip_command_mention_suffix: only strip @bot as a command suffix (#528)
// A command suffix (/stop@opencrabsbot) is stripped for command matching, but
// standalone mentions are preserved so the agent knows it was addressed and
// multi-bot groups keep context.

#[test]
fn strips_at_bot_only_as_command_suffix() {
    assert_eq!(
        strip_command_mention_suffix("/stop@opencrabsbot", "opencrabsbot"),
        "/stop"
    );
    assert_eq!(
        strip_command_mention_suffix("/models@opencrabsbot gpt", "opencrabsbot"),
        "/models gpt"
    );
}

#[test]
fn preserves_standalone_mention_for_the_agent() {
    // The whole point of #528: a standalone mention survives so the agent sees
    // it was addressed.
    assert_eq!(
        strip_command_mention_suffix("hey @opencrabsbot do X", "opencrabsbot"),
        "hey @opencrabsbot do X"
    );
    // Multi-bot: only THIS bot's command suffix goes; the other mention stays.
    assert_eq!(
        strip_command_mention_suffix("hey @opencrabsbot @otherbot do X", "opencrabsbot"),
        "hey @opencrabsbot @otherbot do X"
    );
}

#[test]
fn does_not_strip_a_longer_username_prefix() {
    // @opencrabsbot must not match inside @opencrabsbot2.
    assert_eq!(
        strip_command_mention_suffix("/stop@opencrabsbot2", "opencrabsbot"),
        "/stop@opencrabsbot2"
    );
}

#[test]
fn plain_text_and_bare_commands_are_unchanged() {
    assert_eq!(
        strip_command_mention_suffix("just a message", "opencrabsbot"),
        "just a message"
    );
    assert_eq!(
        strip_command_mention_suffix("/stop", "opencrabsbot"),
        "/stop"
    );
}
