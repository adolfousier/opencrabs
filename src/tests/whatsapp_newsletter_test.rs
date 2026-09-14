//! Pure decision logic for the WhatsApp newsletter poller (#1529):
//! new-post selection, monotonic cursor advancement, silent first-sight
//! baselining, and digest shape. The IO leg (get_messages / send) is
//! compiler-typed composition around these.

use crate::channels::whatsapp::newsletter::{
    Cursor, PostRef, format_digest, next_cursor, select_new,
};

fn post(sid: u64, ts: u64) -> PostRef {
    PostRef { server_id: sid, ts }
}

fn cur(sid: i64) -> Cursor {
    Cursor {
        last_server_id: sid,
        last_ts: 0,
    }
}

#[test]
fn first_sight_selects_nothing_and_baselines_on_newest() {
    let posts = [post(7, 100), post(9, 140), post(8, 120)];
    assert!(select_new(None, &posts).is_empty(), "no replay at opt-in");
    let base = next_cursor(None, &posts);
    assert_eq!(base.last_server_id, 9, "baseline rides the newest post");
    assert_eq!(base.last_ts, 140);
}

#[test]
fn empty_channel_gets_an_explicit_zero_cursor() {
    let c = next_cursor(None, &[]);
    assert_eq!((c.last_server_id, c.last_ts), (0, 0));
    // A zero cursor keeps the next empty page from re-baselining forever,
    // and the first real post is strictly greater than it.
    assert_eq!(select_new(Some(c), &[post(1, 10)]).len(), 1);
}

#[test]
fn new_posts_are_filtered_strictly_and_ordered_oldest_first() {
    let posts = [post(12, 300), post(10, 250), post(11, 280), post(9, 200)];
    let new = select_new(Some(cur(10)), &posts);
    assert_eq!(
        new.iter().map(|p| p.server_id).collect::<Vec<_>>(),
        vec![11, 12],
        "id 10 itself and older must not re-digest; output is chronological"
    );
}

#[test]
fn cursor_advances_monotonically_and_never_regresses() {
    let advanced = next_cursor(Some(cur(11)), &[post(13, 400), post(12, 350)]);
    assert_eq!((advanced.last_server_id, advanced.last_ts), (13, 400));
    // An older page (server hiccup, clock skew) must not drag the id back.
    let held = next_cursor(Some(cur(13)), &[post(12, 350)]);
    assert_eq!(held.last_server_id, 13);
    // No new posts: cursor preserved exactly, including its ts.
    let same = next_cursor(Some(cur(13)), &[]);
    assert_eq!(same, cur(13));
}

#[test]
fn digest_names_channel_counts_and_lists_in_order() {
    let posts = [post(4, 1_800_000_000), post(5, 1_800_003_600)];
    let texts = vec!["first body".to_string(), "second body".to_string()];
    let out = format_digest("Kreb's Newsletter", &posts, &texts);
    assert!(out.contains("Kreb's Newsletter"));
    assert!(out.contains("(2 new)"));
    assert!(out.find("first body").unwrap() < out.find("second body").unwrap());
    let expected = chrono::DateTime::from_timestamp(1_800_000_000, 0)
        .unwrap()
        .format("%Y-%m-%d %H:%M")
        .to_string();
    assert!(
        out.contains(&expected),
        "timestamps render human-readable, got: {out}"
    );
}
