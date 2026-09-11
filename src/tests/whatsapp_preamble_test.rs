//! Regression for #1489: both WhatsApp preambles denied `whatsapp_send`
//! exists ("There is no whatsapp_send tool. Just reply with text.") while
//! `catalog.rs` registered it with 14 actions and the system prompt
//! protected it from the generic-`message`-tool rewrite. A user asking for
//! a photo therefore fell back to generic tools or emitted a bare path.
//!
//! The preambles now mirror Telegram's pattern (`handler.rs`: "Do NOT call
//! telegram_send to deliver your answer. Only use telegram_send for:
//! ...media, polls..."): the auto-delivery contract stays, the tool is
//! acknowledged, and its real use cases are enumerated. These sentinels
//! scan the handler source so a future preamble rewrite cannot silently
//! re-deny a registered tool.

const WHATSAPP_HANDLER: &str = include_str!("../channels/whatsapp/handler.rs");

/// The denial phrase that caused #1489 must not come back in any form.
#[test]
fn denial_phrase_is_gone() {
    assert!(
        !WHATSAPP_HANDLER.contains("There is no whatsapp_send"),
        "whatsapp handler still denies whatsapp_send exists (#1489)"
    );
    assert!(
        !WHATSAPP_HANDLER.contains("There is no `whatsapp_send`"),
        "whatsapp handler still denies whatsapp_send exists (backticked variant)"
    );
}

/// The per-turn channel preamble keeps the auto-delivery contract AND
/// directs media/poll/reaction traffic through whatsapp_send.
#[test]
fn per_turn_preamble_directs_media_through_whatsapp_send() {
    let (_, rest) = WHATSAPP_HANDLER
        .split_once("[Channel: WhatsApp (chat_id: {chat_id})")
        .expect("per-turn channel preamble not found");
    let (preamble, _) = rest
        .split_once("]\\n{agent_input}")
        .expect("per-turn preamble end marker not found");
    assert!(
        preamble.contains("automatically sent to this chat"),
        "auto-delivery contract missing from per-turn preamble"
    );
    assert!(
        preamble.contains("Do NOT call whatsapp_send to deliver your answer"),
        "per-turn preamble must forbid whatsapp_send for plain replies"
    );
    for capability in ["media", "polls", "reactions", "quote-replies"] {
        assert!(
            preamble.contains(capability),
            "per-turn preamble must enumerate {capability} as a whatsapp_send use case"
        );
    }
    assert!(
        preamble.contains("ORDERING: send any files/documents/photos FIRST"),
        "per-turn preamble must carry the media-before-text ordering rule"
    );
}

/// The connect-greeting preamble acknowledges the tool too (the greeting is
/// text-only, but the model must not be told the tool doesn't exist).
#[test]
fn greeting_preamble_names_whatsapp_send() {
    let (_, rest) = WHATSAPP_HANDLER
        .split_once("[Channel: WhatsApp — your text response is automatically sent to this chat")
        .expect("greeting channel preamble not found");
    let (greeting, _) = rest
        .split_once("You have just connected")
        .expect("greeting preamble end marker not found");
    assert!(
        greeting.contains("Do NOT call whatsapp_send to deliver your answer"),
        "greeting preamble must forbid whatsapp_send for the reply itself"
    );
    assert!(
        greeting.contains("media"),
        "greeting preamble must state what whatsapp_send IS for"
    );
}
