//! Structured extracts for inbound types the handler used to drop (#1410, #1483).

use waproto::whatsapp::message::{
    ContactMessage, LiveLocationMessage, LocationMessage, ReactionMessage,
};

use crate::channels::whatsapp::inbound::{
    describe_contact, describe_live_location, describe_location, describe_reaction,
    phone_from_vcard,
};

fn pin(lat: f64, lng: f64) -> LocationMessage {
    LocationMessage {
        degrees_latitude: Some(lat),
        degrees_longitude: Some(lng),
        ..Default::default()
    }
}

#[test]
fn a_dropped_pin_renders_as_coordinates() {
    assert_eq!(
        describe_location(&pin(40.176, -8.410)),
        "[location] 40.176000, -8.410000"
    );
}

#[test]
fn a_named_place_carries_its_name() {
    let mut loc = pin(40.176, -8.410);
    loc.name = Some("Universidade de Coimbra".to_string());
    assert_eq!(
        describe_location(&loc),
        "[location] 40.176000, -8.410000 (Universidade de Coimbra)"
    );
}

#[test]
fn a_named_place_with_an_address_carries_both() {
    let mut loc = pin(1.0, 2.0);
    loc.name = Some("Office".to_string());
    loc.address = Some("12 Some Street".to_string());
    assert_eq!(
        describe_location(&loc),
        "[location] 1.000000, 2.000000 (Office) - 12 Some Street"
    );
}

#[test]
fn blank_labels_are_not_rendered_as_empty_brackets() {
    let mut loc = pin(1.0, 2.0);
    loc.name = Some("   ".to_string());
    loc.address = Some(String::new());
    assert_eq!(describe_location(&loc), "[location] 1.000000, 2.000000");
}

#[test]
fn a_location_missing_coordinates_still_renders() {
    // A malformed pin must not make the whole message vanish again.
    assert_eq!(
        describe_location(&LocationMessage::default()),
        "[location] 0.000000, 0.000000"
    );
}

#[test]
fn a_live_location_is_labelled_as_live() {
    let loc = LiveLocationMessage {
        degrees_latitude: Some(38.7),
        degrees_longitude: Some(-9.1),
        ..Default::default()
    };
    assert_eq!(
        describe_live_location(&loc),
        "[live location] 38.700000, -9.100000"
    );
}

#[test]
fn a_live_location_caption_is_appended() {
    let loc = LiveLocationMessage {
        degrees_latitude: Some(38.7),
        degrees_longitude: Some(-9.1),
        caption: Some("on my way".to_string()),
        ..Default::default()
    };
    assert_eq!(
        describe_live_location(&loc),
        "[live location] 38.700000, -9.100000 - on my way"
    );
}

#[test]
fn a_whatsapp_vcard_yields_its_phone_number() {
    let vcard = "BEGIN:VCARD\nVERSION:3.0\nFN:Ada\nTEL;type=CELL;waid=15551234567:+1 555 123 4567\nEND:VCARD";
    assert_eq!(phone_from_vcard(vcard).as_deref(), Some("+1 555 123 4567"));
}

#[test]
fn a_vcard_with_a_plain_tel_line_works_too() {
    assert_eq!(
        phone_from_vcard("BEGIN:VCARD\nTEL:+351911111111\nEND:VCARD").as_deref(),
        Some("+351911111111")
    );
}

#[test]
fn a_vcard_without_a_tel_line_yields_nothing() {
    assert_eq!(phone_from_vcard("BEGIN:VCARD\nFN:Ada\nEND:VCARD"), None);
}

#[test]
fn an_empty_tel_value_yields_nothing() {
    assert_eq!(phone_from_vcard("TEL;type=CELL:"), None);
}

#[test]
fn a_contact_card_renders_name_and_number() {
    let contact = ContactMessage {
        display_name: Some("Ada Lovelace".to_string()),
        vcard: Some("BEGIN:VCARD\nTEL;type=CELL:+441234567890\nEND:VCARD".to_string()),
        ..Default::default()
    };
    assert_eq!(
        describe_contact(&contact),
        "[contact] Ada Lovelace - +441234567890"
    );
}

#[test]
fn a_contact_card_without_a_number_still_names_the_person() {
    let contact = ContactMessage {
        display_name: Some("Ada Lovelace".to_string()),
        ..Default::default()
    };
    assert_eq!(describe_contact(&contact), "[contact] Ada Lovelace");
}

#[test]
fn a_nameless_contact_card_is_still_surfaced() {
    assert_eq!(
        describe_contact(&ContactMessage::default()),
        "[contact] unnamed"
    );
}

#[test]
fn an_inbound_reaction_names_its_emoji_and_target() {
    let reaction = ReactionMessage {
        text: Some("👍".to_string()),
        key: Some(waproto::whatsapp::MessageKey {
            id: Some("3EB0ABC".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(
        describe_reaction(&reaction).as_deref(),
        Some("[reaction] 👍 on message 3EB0ABC")
    );
}

#[test]
fn a_removed_reaction_is_not_reported_as_approval() {
    // WhatsApp encodes removal as an empty emoji. Surfacing that as a bare
    // reaction would read to the agent as a thumbs-up that was taken back.
    let reaction = ReactionMessage {
        text: Some(String::new()),
        ..Default::default()
    };
    assert_eq!(describe_reaction(&reaction), None);
    assert_eq!(describe_reaction(&ReactionMessage::default()), None);
}

#[test]
fn a_reaction_without_a_key_still_reports_the_emoji() {
    let reaction = ReactionMessage {
        text: Some("🔥".to_string()),
        ..Default::default()
    };
    assert_eq!(
        describe_reaction(&reaction).as_deref(),
        Some("[reaction] 🔥 on message unknown message")
    );
}
