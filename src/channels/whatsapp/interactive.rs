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
        body: Some(Body {
            text: Some(body.to_string()),
        }),
        footer: footer.map(|f| {
            Box::new(Footer {
                text: Some(f.to_string()),
                ..Default::default()
            })
        }),
        interactive_message: Some(InteractiveOneof::NativeFlowMessage(NativeFlowMessage {
            buttons: flow_buttons,
            message_version: Some(NATIVE_FLOW_VERSION),
            ..Default::default()
        })),
        ..Default::default()
    };

    let inner = waproto::whatsapp::Message {
        message_context_info: Some(Box::new(waproto::whatsapp::MessageContextInfo {
            device_list_metadata_version: Some(2),
            ..Default::default()
        })),
        interactive_message: Some(Box::new(interactive)),
        ..Default::default()
    };

    waproto::whatsapp::Message {
        view_once_message: Some(Box::new(waproto::whatsapp::message::FutureProofMessage {
            message: Some(Box::new(inner)),
        })),
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
    if let Some(resp) = msg.interactive_response_message.as_ref()
        && let Some(ResponseOneof::NativeFlowResponseMessage(flow)) =
            resp.interactive_response_message.as_ref()
        && let Some(params) = flow.params_json.as_deref()
        && let Some(id) = id_from_params(params)
    {
        return Some(id);
    }
    msg.buttons_response_message
        .as_ref()
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
