//! Inbound poll vote resolution (#1482).

use crate::channels::whatsapp::poll::{POLL_CAPACITY, PollOptions, describe_vote, resolve_options};
use wacore::poll::compute_option_hash;

fn hash(option: &str) -> Vec<u8> {
    compute_option_hash(option).to_vec()
}

fn options() -> Vec<String> {
    vec!["Rust".to_string(), "Zig".to_string(), "C".to_string()]
}

#[test]
fn a_single_vote_resolves_to_its_label() {
    assert_eq!(resolve_options(&[hash("Zig")], &options()), vec!["Zig"]);
}

#[test]
fn a_multi_vote_resolves_every_label() {
    let selected = vec![hash("C"), hash("Rust")];
    assert_eq!(
        resolve_options(&selected, &options()),
        vec!["Rust", "C"],
        "order follows the poll, not the vote, so two voters read identically"
    );
}

#[test]
fn an_unknown_hash_is_dropped_rather_than_shown_as_hex() {
    let selected = vec![hash("Rust"), hash("something we never offered")];
    assert_eq!(resolve_options(&selected, &options()), vec!["Rust"]);
}

#[test]
fn a_vote_for_nothing_resolves_to_nothing() {
    assert!(resolve_options(&[], &options()).is_empty());
}

#[test]
fn a_poll_with_no_options_resolves_nothing() {
    assert!(resolve_options(&[hash("Rust")], &[]).is_empty());
}

#[test]
fn option_hashing_is_what_the_lib_does() {
    // If this ever stops matching, votes silently resolve to nothing rather
    // than failing loudly, so it is worth pinning.
    assert_eq!(compute_option_hash("Rust").len(), 32);
    assert_ne!(compute_option_hash("Rust"), compute_option_hash("rust"));
}

#[test]
fn a_vote_renders_with_the_voter_and_the_choices() {
    assert_eq!(
        describe_vote("15551234", &["Rust".to_string(), "C".to_string()]).as_deref(),
        Some("[poll vote] 15551234 chose: Rust, C")
    );
}

#[test]
fn a_cleared_vote_renders_nothing() {
    // The voter deselected everything. Reporting "chose:" with an empty list
    // would read as a vote that was never cast.
    assert_eq!(describe_vote("15551234", &[]), None);
}

#[tokio::test]
async fn poll_options_round_trip() {
    let polls = PollOptions::default();
    polls.remember("3EB0POLL", options()).await;
    assert_eq!(polls.get("3EB0POLL").await, Some(options()));
}

#[tokio::test]
async fn an_unknown_poll_has_no_options() {
    let polls = PollOptions::default();
    assert!(polls.get("3EB0MISSING").await.is_none());
}

#[tokio::test]
async fn an_empty_id_or_option_list_is_not_stored() {
    let polls = PollOptions::default();
    polls.remember("", options()).await;
    polls.remember("3EB0EMPTY", Vec::new()).await;
    assert_eq!(polls.len().await, 0);
}

#[tokio::test]
async fn the_oldest_poll_is_evicted_once_the_window_is_full() {
    let polls = PollOptions::default();
    for i in 0..=POLL_CAPACITY {
        polls.remember(format!("poll-{i}"), options()).await;
    }
    assert_eq!(polls.len().await, POLL_CAPACITY);
    assert!(polls.get("poll-0").await.is_none());
    assert!(polls.get(&format!("poll-{POLL_CAPACITY}")).await.is_some());
}

#[tokio::test]
async fn re_remembering_a_poll_replaces_its_options() {
    let polls = PollOptions::default();
    polls.remember("p", vec!["old".to_string()]).await;
    polls.remember("p", vec!["new".to_string()]).await;
    assert_eq!(polls.get("p").await, Some(vec!["new".to_string()]));
    assert_eq!(polls.len().await, 1);
}
