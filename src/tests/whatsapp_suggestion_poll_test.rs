//! Suggestion polls on WhatsApp (#1616).
//!
//! Native buttons cap at three, so any larger suggestion set fell to a
//! numbered list and the user had to type. A single-choice poll is the only
//! other one-tap surface WhatsApp offers, and wiring it up needs two halves:
//! choosing the poll for the sets the card cannot take, and turning the
//! resulting vote back into a selection. A vote names its option by LABEL, so
//! the numeric selector can never match one.
//!
//! Whether a phone draws the poll cannot be tested here. What is pinned is the
//! routing: which surface a given set picks, and that a vote consumes the
//! pending set exactly once.

use crate::brain::tools::suggest_options::MAX_OPTIONS;
use crate::channels::whatsapp::WhatsAppState;
use crate::channels::whatsapp::interactive::{
    MAX_BUTTONS, MAX_POLL_OPTIONS, SuggestionSurface, suggestion_card_fits, suggestion_poll_fits,
    suggestion_surface,
};
use uuid::Uuid;

#[test]
fn the_card_keeps_what_it_can_take() {
    // Sets within the button cap satisfy BOTH predicates; the card wins
    // because it carries the body text and the typed instructions with it.
    for n in 1..=MAX_BUTTONS {
        assert_eq!(suggestion_surface(n, true), SuggestionSurface::Card, "{n}");
    }
}

#[test]
fn sets_past_the_button_cap_get_a_poll() {
    // The whole point of #1616: 4 to 8 options used to fall to typed numbers.
    for n in (MAX_BUTTONS + 1)..=MAX_OPTIONS {
        assert_eq!(
            suggestion_surface(n, true),
            SuggestionSurface::Poll,
            "{n} options is past the {MAX_BUTTONS}-button cap and must render as a poll"
        );
    }
}

#[test]
fn a_single_option_is_not_a_poll() {
    // WhatsApp rejects a one-option poll, and one option is not a choice.
    assert!(!suggestion_poll_fits(1));
    assert!(suggestion_card_fits(1), "one option still fits a card");
}

#[test]
fn an_oversized_set_falls_back_to_text() {
    assert_eq!(
        suggestion_surface(MAX_POLL_OPTIONS + 1, true),
        SuggestionSurface::Text
    );
}

#[test]
fn an_opted_out_owner_still_gets_the_numbered_list() {
    // interactive_buttons is opt-in; nothing renders natively without it.
    for n in 1..=MAX_OPTIONS {
        assert_eq!(suggestion_surface(n, false), SuggestionSurface::Text, "{n}");
    }
}

#[tokio::test]
async fn a_vote_label_selects_the_suggestion() {
    let state = WhatsAppState::new();
    let sid = Uuid::new_v4();
    state
        .set_pending_followups(
            sid,
            vec![
                "Run the tests".to_string(),
                "Show me the diff".to_string(),
                "Ship it".to_string(),
                "Roll back".to_string(),
            ],
        )
        .await;

    let picked = state.take_followup_by_label(sid, "Show me the diff").await;
    assert_eq!(picked.as_deref(), Some("Show me the diff"));

    // Consumed: a second vote on the same poll must not re-fire it.
    assert!(
        state.take_followup_by_label(sid, "Ship it").await.is_none(),
        "a selection consumes the whole set"
    );
}

#[tokio::test]
async fn vote_matching_survives_the_round_trip_through_the_poll_proto() {
    let state = WhatsAppState::new();
    let sid = Uuid::new_v4();
    state
        .set_pending_followups(sid, vec!["Deploy to prod".to_string()])
        .await;

    assert_eq!(
        state
            .take_followup_by_label(sid, "  deploy to PROD  ")
            .await
            .as_deref(),
        Some("Deploy to prod"),
        "matching is trimmed and case-insensitive"
    );
}

#[tokio::test]
async fn a_label_that_is_not_on_offer_selects_nothing() {
    let state = WhatsAppState::new();
    let sid = Uuid::new_v4();
    state
        .set_pending_followups(sid, vec!["Run the tests".to_string()])
        .await;

    assert!(
        state
            .take_followup_by_label(sid, "Delete production")
            .await
            .is_none()
    );
    assert!(state.take_followup_by_label(sid, "").await.is_none());
    // The set survives a miss, so the real option can still be picked.
    assert_eq!(
        state.take_followup_by_label(sid, "Run the tests").await,
        Some("Run the tests".to_string())
    );
}

#[test]
fn the_vote_carries_its_labels_rather_than_re_parsing_the_prose() {
    // decode_vote returns the labels alongside the rendered line. Splitting
    // "[poll vote] X chose: Y" back apart would break on any wording change.
    const POLL: &str = include_str!("../channels/whatsapp/poll.rs");
    assert!(
        POLL.contains("pub(crate) struct Vote {") && POLL.contains("pub chosen: Vec<String>,"),
        "the decoded vote must expose its labels (#1616)"
    );
}
