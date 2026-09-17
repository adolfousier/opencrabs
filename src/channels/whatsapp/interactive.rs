//! Native-flow interactive buttons (#1411).
//!
//! This channel shipped buttons once and removed them: commit 2f15f1d1 tore
//! `ButtonsMessage` out of the approval flow because modern clients silently
//! never render it, and the comment saying so is still in `handler.rs`. So the
//! bar for putting buttons back is higher than "the field exists".
//!
//! `NativeFlowMessage` is a genuinely different path from the one that failed:
//! it rides inside `InteractiveMessage`, which is what WhatsApp's own Business
//! surfaces use, and it is what every current unofficial client sends. That
//! makes it worth trying. It does NOT make it proven - whether a given client
//! draws the card can only be settled by sending one to a real phone. Two
//! consequences shape the code below:
//!
//! * The approval flow keeps its plain-text prompt unless the owner turns
//!   `interactive_buttons` on. An approval prompt that does not render is a
//!   safety-critical message the user cannot answer, so this does not default
//!   to the unproven path.
//! * The body text always carries the same instructions the text-only prompt
//!   would have. A card that renders without its buttons still tells the
//!   reader what to type.
//!
//! Tap parsing is always on, whatever the flag says: it costs nothing, and a
//! tap that arrived is evidence the card rendered.

/// One quick-reply button.
pub(crate) struct Button {
    /// Stable id echoed back in the tap. Never shown to the user.
    pub id: String,
    /// Text on the button face.
    pub label: String,
}

impl Button {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

/// WhatsApp draws at most three quick replies. A fourth is not an error on the
/// wire, it just never appears, which is worse than refusing it here.
pub(crate) const MAX_BUTTONS: usize = 3;

/// `messageVersion` for a quick-reply flow. Higher versions are for real
/// Flows (forms, native surfaces), which this is not.
const NATIVE_FLOW_VERSION: i32 = 1;

/// Build the outbound interactive message.
///
/// The card is wrapped in `viewOnceMessage`, which is not about disappearing
/// media: it is the envelope WhatsApp's clients expect an `InteractiveMessage`
/// to arrive in, and an unwrapped one is dropped on the floor. The wrapper
/// also carries `deviceListMetadataVersion = 2`, which multi-device clients
/// require before they will render anything inside it.
///
/// Buttons past [`MAX_BUTTONS`] are dropped here rather than sent and silently
/// ignored, so the caller's `buttons.len()` matches what the user sees.
pub(crate) fn build(
    body: &str,
    footer: Option<&str>,
    buttons: &[Button],
) -> waproto::whatsapp::Message {
    use waproto::whatsapp::message::interactive_message::native_flow_message::NativeFlowButton;
    use waproto::whatsapp::message::interactive_message::{
        Body, Footer, InteractiveMessage as InteractiveOneof, NativeFlowMessage,
    };

    let flow_buttons: Vec<NativeFlowButton> = buttons
        .iter()
        .take(MAX_BUTTONS)
        .map(|b| NativeFlowButton {
            name: Some("quick_reply".to_string()),
            button_params_json: Some(button_params(&b.id, &b.label)),
        })
        .collect();

    let interactive = waproto::whatsapp::message::InteractiveMessage {
        body: Body {
            text: Some(body.to_string()),
        }
        .into(),
        footer: footer
            .map(|f| Footer {
                text: Some(f.to_string()),
                ..Default::default()
            })
            .into(),
        interactive_message: Some(InteractiveOneof::NativeFlowMessage(Box::new(
            NativeFlowMessage {
                buttons: flow_buttons,
                message_version: Some(NATIVE_FLOW_VERSION),
                ..Default::default()
            },
        ))),
        ..Default::default()
    };

    let inner = waproto::whatsapp::Message {
        message_context_info: waproto::whatsapp::MessageContextInfo {
            device_list_metadata_version: Some(2),
            ..Default::default()
        }
        .into(),
        interactive_message: interactive.into(),
        ..Default::default()
    };

    waproto::whatsapp::Message {
        view_once_message: waproto::whatsapp::message::FutureProofMessage {
            message: inner.into(),
        }
        .into(),
        ..Default::default()
    }
}

/// The `buttonParamsJson` blob for one quick reply.
///
/// Built with `serde_json` rather than `format!` so a label containing a quote
/// produces valid JSON instead of a button the client cannot parse - and a
/// label is user-facing text, so quotes in it are ordinary, not exotic.
fn button_params(id: &str, label: &str) -> String {
    serde_json::json!({ "display_text": label, "id": id }).to_string()
}

/// Extract the id of a tapped button, from either response shape.
///
/// `interactiveResponseMessage` is what a native-flow tap returns; the params
/// come back as a JSON blob rather than a field, so the id is read out of it.
/// `buttonsResponseMessage` is the legacy shape 2f15f1d1 removed the SENDING
/// side of - it is still parsed, because a card sent by something other than
/// us (a Business account, another tool) may answer that way, and refusing to
/// read it would drop a tap the user really made.
pub(crate) fn parse_tap(msg: &waproto::whatsapp::Message) -> Option<String> {
    use waproto::whatsapp::message::interactive_response_message::InteractiveResponseMessage as ResponseOneof;
    if let Some(resp) = msg.interactive_response_message.as_option()
        && let Some(ResponseOneof::NativeFlowResponseMessage(flow)) =
            resp.interactive_response_message.as_ref()
        && let Some(params) = flow.params_json.as_deref()
        && let Some(id) = id_from_params(params)
    {
        return Some(id);
    }
    msg.buttons_response_message
        .as_option()
        .and_then(|b| b.selected_button_id.as_deref())
        .map(str::to_string)
}

/// Pull `id` out of a tap's params JSON.
///
/// Clients are not consistent about what else they put in there, so this reads
/// the one field it needs and ignores the rest. Malformed JSON yields `None`
/// rather than a panic: a tap we cannot read is a tap we did not receive, and
/// the text path still answers.
fn id_from_params(params: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(params)
        .ok()?
        .get("id")?
        .as_str()
        .map(str::to_string)
}

/// Decode a suggestion-card tap id (`wa_suggest_N`) into its 1-based number
/// (#1411). Any other id (approval taps, foreign payloads, zero, junk)
/// yields `None` so the ordinary message path stays untouched. Kept pure so
/// the routing contract is testable without a live socket.
pub(crate) fn parse_suggestion_tap(id: &str) -> Option<usize> {
    let n: usize = id.strip_prefix("wa_suggest_")?.parse().ok()?;
    (n >= 1).then_some(n)
}

/// Whether a suggestion set may render as a native-flow card: the opt-in flag
/// is checked by the caller; this enforces the button cap, because a truncated
/// card would silently drop selectable options. Pure for the same reason as
/// [`parse_suggestion_tap`].
pub(crate) fn suggestion_card_fits(count: usize) -> bool {
    (1..=MAX_BUTTONS).contains(&count)
}

/// WhatsApp's own cap on poll options. `suggest_options` tops out at 8, so
/// this is headroom rather than a real boundary, but a poll built past it is
/// rejected by the server.
pub(crate) const MAX_POLL_OPTIONS: usize = 12;

/// Whether a suggestion set may render as a single-choice poll (#1616).
///
/// The poll is the one-tap surface for sets too big for the button cap. A
/// single option is not a choice and WhatsApp rejects a one-option poll. Sets
/// that also fit a card are accepted here; [`suggestion_surface`] decides
/// between them. Pure for the same reason as [`parse_suggestion_tap`].
pub(crate) fn suggestion_poll_fits(count: usize) -> bool {
    (2..=MAX_POLL_OPTIONS).contains(&count)
}

/// Where a suggestion set renders (#1411, #1616).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SuggestionSurface {
    /// Native-flow card: one tap, up to [`MAX_BUTTONS`] options.
    Card,
    /// Single-choice poll: one tap, for sets past the button cap.
    Poll,
    /// Numbered list. Always selectable by typing a number, so it is the
    /// fallback for an opted-out owner, an oversized set, or a failed send.
    Text,
}

/// Pick the surface for `count` options.
///
/// Card first: it carries the body text and the instructions, so it degrades
/// better on a client that renders neither. The poll only claims what the card
/// cannot take. Kept pure and out of the handler so the routing is testable
/// without a live socket.
pub(crate) fn suggestion_surface(count: usize, interactive: bool) -> SuggestionSurface {
    if !interactive {
        return SuggestionSurface::Text;
    }
    if suggestion_card_fits(count) {
        SuggestionSurface::Card
    } else if suggestion_poll_fits(count) {
        SuggestionSurface::Poll
    } else {
        SuggestionSurface::Text
    }
}
