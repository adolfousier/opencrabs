//! Native-flow interactive buttons (#1411).
//!
//! Whether a phone DRAWS the card cannot be tested here, and nothing below
//! pretends otherwise. What is pinned is the wire shape: the envelope a client
//! needs before it will even consider rendering, and the tap parsing that has
//! to work for a rendered card to be useful.

use crate::channels::whatsapp::interactive::{Button, MAX_BUTTONS, build, parse_tap};

/// Reach through the view-once envelope to the card itself.
fn card(msg: &waproto::whatsapp::Message) -> &waproto::whatsapp::message::InteractiveMessage {
    msg.view_once_message
        .as_ref()
        .expect("interactive messages must be wrapped in viewOnceMessage")
        .message
        .as_ref()
        .expect("wrapper carries the real message")
        .interactive_message
        .as_ref()
        .expect("card present")
}

fn flow(
    msg: &waproto::whatsapp::Message,
) -> &waproto::whatsapp::message::interactive_message::NativeFlowMessage {
    use waproto::whatsapp::message::interactive_message::InteractiveMessage as Oneof;
    match card(msg).interactive_message.as_ref() {
        Some(Oneof::NativeFlowMessage(f)) => f,
        _ => panic!("expected a native-flow card"),
    }
}

fn approval_buttons() -> Vec<Button> {
    vec![
        Button::new("wa_approve_yes", "Yes"),
        Button::new("wa_approve_no", "No"),
    ]
}

#[test]
fn the_card_is_wrapped_in_view_once() {
    // Not about disappearing media: an unwrapped InteractiveMessage is dropped
    // on the floor by the clients that would otherwise render it.
    let msg = build("approve?", None, &approval_buttons());
    assert!(msg.view_once_message.is_some());
    assert!(
        msg.interactive_message.is_none(),
        "the card belongs inside the wrapper, not beside it"
    );
}

#[test]
fn the_wrapper_declares_multi_device() {
    // Multi-device clients refuse to render the contents without it.
    let inner = build("approve?", None, &approval_buttons())
        .view_once_message
        .unwrap()
        .message
        .unwrap();
    assert_eq!(
        inner
            .message_context_info
            .as_ref()
            .and_then(|c| c.device_list_metadata_version),
        Some(2)
    );
}

#[test]
fn the_body_carries_the_full_prompt() {
    // The rendering risk this whole design is built around: if the card shows
    // but the buttons do not, the text still has to tell the reader what to do.
    let msg = build("Reply yes or no.", None, &approval_buttons());
    assert_eq!(
        card(&msg).body.as_ref().and_then(|b| b.text.as_deref()),
        Some("Reply yes or no.")
    );
}

#[test]
fn a_footer_is_optional() {
    let without = build("body", None, &approval_buttons());
    assert!(card(&without).footer.is_none());

    let with = build("body", Some("tap or type"), &approval_buttons());
    assert_eq!(
        card(&with).footer.as_ref().and_then(|f| f.text.as_deref()),
        Some("tap or type")
    );
}

#[test]
fn every_button_is_a_quick_reply_carrying_its_id_and_label() {
    let msg = build("body", None, &approval_buttons());
    let buttons = &flow(&msg).buttons;
    assert_eq!(buttons.len(), 2);

    assert_eq!(buttons[0].name.as_deref(), Some("quick_reply"));
    let params: serde_json::Value =
        serde_json::from_str(buttons[0].button_params_json.as_deref().unwrap()).unwrap();
    assert_eq!(params["id"], "wa_approve_yes");
    assert_eq!(params["display_text"], "Yes");
}

#[test]
fn a_label_containing_a_quote_still_produces_valid_json() {
    // Labels are user-facing text, so a quote in one is ordinary. Built with
    // format! it would produce a blob the client cannot parse, and the button
    // would vanish with no error anywhere.
    let msg = build("body", None, &[Button::new("id", r#"Say "yes""#)]);
    let raw = flow(&msg).buttons[0].button_params_json.as_deref().unwrap();
    let params: serde_json::Value = serde_json::from_str(raw).expect("valid JSON");
    assert_eq!(params["display_text"], r#"Say "yes""#);
}

#[test]
fn buttons_past_the_cap_are_dropped_here_rather_than_on_the_wire() {
    // WhatsApp draws three. A fourth is not a wire error, it just never
    // appears, so dropping it here keeps len() honest for the caller.
    let many: Vec<Button> = (0..6)
        .map(|i| Button::new(format!("id{i}"), format!("B{i}")))
        .collect();
    assert_eq!(flow(&build("body", None, &many)).buttons.len(), MAX_BUTTONS);
}

#[test]
fn a_card_with_no_buttons_is_still_well_formed() {
    // Degenerate, but it must not panic: the body alone is a usable message.
    let msg = build("body only", None, &[]);
    assert!(flow(&msg).buttons.is_empty());
}

#[test]
fn a_native_flow_tap_yields_its_id() {
    use waproto::whatsapp::message::InteractiveResponseMessage;
    use waproto::whatsapp::message::interactive_response_message::{
        InteractiveResponseMessage as Oneof, NativeFlowResponseMessage,
    };

    let msg = waproto::whatsapp::Message {
        interactive_response_message: Some(Box::new(InteractiveResponseMessage {
            interactive_response_message: Some(Oneof::NativeFlowResponseMessage(
                NativeFlowResponseMessage {
                    name: Some("quick_reply".to_string()),
                    params_json: Some(
                        r#"{"display_text":"Yes","id":"wa_approve_yes"}"#.to_string(),
                    ),
                    ..Default::default()
                },
            )),
            ..Default::default()
        })),
        ..Default::default()
    };
    assert_eq!(parse_tap(&msg).as_deref(), Some("wa_approve_yes"));
}

#[test]
fn a_legacy_buttons_tap_is_still_read() {
    // 2f15f1d1 removed the SENDING side of this shape, not the receiving side.
    // A card sent by a Business account or another tool may answer this way,
    // and refusing to read it would drop a tap the user really made.
    let msg = waproto::whatsapp::Message {
        buttons_response_message: Some(Box::new(
            waproto::whatsapp::message::ButtonsResponseMessage {
                selected_button_id: Some("wa_approve_no".to_string()),
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    assert_eq!(parse_tap(&msg).as_deref(), Some("wa_approve_no"));
}

#[test]
fn an_ordinary_message_is_not_a_tap() {
    let msg = waproto::whatsapp::Message {
        conversation: Some("yes".to_string()),
        ..Default::default()
    };
    assert_eq!(parse_tap(&msg), None);
}

#[test]
fn unreadable_tap_params_yield_nothing_rather_than_panicking() {
    use waproto::whatsapp::message::InteractiveResponseMessage;
    use waproto::whatsapp::message::interactive_response_message::{
        InteractiveResponseMessage as Oneof, NativeFlowResponseMessage,
    };

    // Clients are not consistent about what else rides in this blob. A tap we
    // cannot read is a tap we did not receive; the text path still answers.
    for params in ["not json at all", "{}", r#"{"id":42}"#, r#"{"other":"x"}"#] {
        let msg = waproto::whatsapp::Message {
            interactive_response_message: Some(Box::new(InteractiveResponseMessage {
                interactive_response_message: Some(Oneof::NativeFlowResponseMessage(
                    NativeFlowResponseMessage {
                        params_json: Some(params.to_string()),
                        ..Default::default()
                    },
                )),
                ..Default::default()
            })),
            ..Default::default()
        };
        assert_eq!(parse_tap(&msg), None, "params: {params}");
    }
}

#[test]
fn interactive_buttons_are_off_by_default() {
    // The rail that matters: an approval prompt that does not render is one
    // the owner cannot answer, so the unproven path is never the default.
    assert!(
        !crate::config::Config::default()
            .channels
            .whatsapp
            .interactive_buttons
    );
}
