//! Local blocklist mirror (#1487).

use crate::channels::whatsapp::blocklist::{Blocklist, user_part};

#[test]
fn user_part_strips_the_server() {
    assert_eq!(user_part("15551234@s.whatsapp.net"), "15551234");
}

#[test]
fn user_part_strips_the_device_suffix() {
    // A block applies to the whole account. Leaving the device id on would let
    // the same contact walk straight past the guard from a second device.
    assert_eq!(user_part("15551234@s.whatsapp.net/3"), "15551234");
}

#[test]
fn user_part_handles_a_lid() {
    assert_eq!(user_part("236927743742100@lid"), "236927743742100");
}

#[test]
fn user_part_passes_through_a_bare_user() {
    assert_eq!(user_part("15551234"), "15551234");
}

#[tokio::test]
async fn a_fresh_mirror_blocks_nobody() {
    let list = Blocklist::default();
    assert!(!list.contains("15551234@s.whatsapp.net").await);
    assert_eq!(list.len().await, 0);
}

#[tokio::test]
async fn an_inserted_contact_is_blocked() {
    let list = Blocklist::default();
    list.insert("15551234@s.whatsapp.net").await;
    assert!(list.contains("15551234@s.whatsapp.net").await);
}

#[tokio::test]
async fn a_blocked_contact_cannot_slip_through_on_another_device() {
    let list = Blocklist::default();
    list.insert("15551234@s.whatsapp.net").await;
    assert!(
        list.contains("15551234@s.whatsapp.net/7").await,
        "the device suffix must not defeat the guard"
    );
}

#[tokio::test]
async fn removing_a_contact_unblocks_it() {
    let list = Blocklist::default();
    list.insert("15551234@s.whatsapp.net").await;
    list.remove("15551234@s.whatsapp.net").await;
    assert!(!list.contains("15551234@s.whatsapp.net").await);
}

#[tokio::test]
async fn removing_by_a_different_device_form_still_unblocks() {
    let list = Blocklist::default();
    list.insert("15551234@s.whatsapp.net/2").await;
    list.remove("15551234@s.whatsapp.net").await;
    assert!(!list.contains("15551234@s.whatsapp.net").await);
}

#[tokio::test]
async fn replace_swaps_the_set_rather_than_merging() {
    // The connect-time refresh is authoritative: a contact the owner unblocked
    // from their phone must not survive in the mirror.
    let list = Blocklist::default();
    list.insert("11111111@s.whatsapp.net").await;
    list.replace(["22222222@s.whatsapp.net".to_string()]).await;

    assert!(!list.contains("11111111@s.whatsapp.net").await);
    assert!(list.contains("22222222@s.whatsapp.net").await);
    assert_eq!(list.len().await, 1);
}

#[tokio::test]
async fn replace_with_nothing_clears_the_mirror() {
    let list = Blocklist::default();
    list.insert("11111111@s.whatsapp.net").await;
    list.replace(Vec::new()).await;
    assert_eq!(list.len().await, 0);
}

#[tokio::test]
async fn replace_normalises_device_suffixes() {
    let list = Blocklist::default();
    list.replace(["15551234@s.whatsapp.net/4".to_string()])
        .await;
    assert!(list.contains("15551234@s.whatsapp.net").await);
}

#[tokio::test]
async fn an_unrelated_contact_is_never_blocked() {
    let list = Blocklist::default();
    list.insert("15551234@s.whatsapp.net").await;
    assert!(!list.contains("15559999@s.whatsapp.net").await);
}

// ── Both addresses one stanza can carry (#1531) ──────────────────────────

#[tokio::test]
async fn a_blocked_pn_is_caught_when_the_stanza_arrives_by_lid() {
    // THE bug. A modern 1:1 DM is addressed by the opaque LID, while this
    // mirror holds phone numbers. Checking only the address the stanza was
    // sent to let a blocked contact message freely from a LID — past the one
    // guard whose whole job is stopping them.
    let list = Blocklist::default();
    list.insert("15551234@s.whatsapp.net").await;

    assert!(
        !list.contains("236927743742100@lid").await,
        "the LID alone is not in the mirror, which is why sender-only failed"
    );
    assert!(
        list.blocks_either("236927743742100@lid", Some("15551234@s.whatsapp.net"))
            .await,
        "the PN riding alongside the LID must still block"
    );
}

#[tokio::test]
async fn a_blocked_sender_is_caught_with_no_alt_at_all() {
    // Group stanzas and older clients carry no sender_alt; the guard has to
    // keep working on the address it does have.
    let list = Blocklist::default();
    list.insert("15551234@s.whatsapp.net").await;
    assert!(list.blocks_either("15551234@s.whatsapp.net", None).await);
}

#[tokio::test]
async fn an_unblocked_pair_passes_through() {
    // Neither half matching must not be an accidental block: over-blocking
    // silently drops a stranger's first message with no trace.
    let list = Blocklist::default();
    list.insert("15551234@s.whatsapp.net").await;
    assert!(
        !list
            .blocks_either("999@lid", Some("15559999@s.whatsapp.net"))
            .await
    );
}

#[tokio::test]
async fn either_address_alone_is_enough_to_block() {
    // The mirror can hold a LID directly, since `replace` takes whatever the
    // server returned. Both directions have to work.
    let list = Blocklist::default();
    list.insert("236927743742100@lid").await;
    assert!(
        list.blocks_either("236927743742100@lid", Some("15551234@s.whatsapp.net"))
            .await,
        "matched on the sender"
    );
    assert!(
        list.blocks_either("15551234@s.whatsapp.net", Some("236927743742100@lid"))
            .await,
        "matched on the alt"
    );
}

#[tokio::test]
async fn a_device_suffix_does_not_defeat_the_pair_check() {
    // Same normalisation as `contains`: a block is on the account, not a
    // device, and both halves have to honour that.
    let list = Blocklist::default();
    list.insert("15551234@s.whatsapp.net").await;
    assert!(
        list.blocks_either("236927743742100@lid/3", Some("15551234@s.whatsapp.net/7"))
            .await
    );
}
