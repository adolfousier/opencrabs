//! Notify receipts (fork #50): queued to injected, target-scoped stamping, idempotent drain.

use crate::brain::agent::service::notify_receipts::*;
use uuid::Uuid;

#[test]
fn receipt_lifecycle_queued_then_injected() {
    let (id, target) = (Uuid::new_v4(), Uuid::new_v4());
    record_queued(id, target);
    let receipt = status(id).expect("receipt recorded");
    assert_eq!(receipt.state, ReceiptState::Queued);
    assert_eq!(receipt.target, target);
    assert!(receipt.injected_at.is_none());

    let stamped = mark_injected_for_target(target);
    assert_eq!(stamped, 1);
    let receipt = status(id).expect("receipt survives stamping");
    assert_eq!(receipt.state, ReceiptState::Injected);
    assert!(receipt.injected_at.is_some());
}

#[test]
fn injection_stamp_is_target_scoped() {
    let (id_a, id_b) = (Uuid::new_v4(), Uuid::new_v4());
    let (target_a, target_b) = (Uuid::new_v4(), Uuid::new_v4());
    record_queued(id_a, target_a);
    record_queued(id_b, target_b);

    assert_eq!(mark_injected_for_target(target_a), 1);
    assert_eq!(status(id_a).unwrap().state, ReceiptState::Injected);
    assert_eq!(status(id_b).unwrap().state, ReceiptState::Queued);
}

#[test]
fn status_of_unknown_id_is_none() {
    assert!(status(Uuid::new_v4()).is_none());
}

#[test]
fn drain_is_idempotent_per_receipt() {
    let (id, target) = (Uuid::new_v4(), Uuid::new_v4());
    record_queued(id, target);
    assert_eq!(mark_injected_for_target(target), 1);
    // A second drain (next tool iteration) must not resurrect or
    // double-count the already-injected receipt.
    assert_eq!(mark_injected_for_target(target), 0);
    let receipt = status(id).unwrap();
    assert_eq!(receipt.state, ReceiptState::Injected);
}

#[test]
fn reserve_is_first_sight_only() {
    // #199 idempotency leg: the first reservation OWNS the notify. A second
    // sight of the same id is a retry whose first response was lost, so it
    // must be refused — the caller reports the prior outcome instead of
    // delivering a second copy.
    let (id, target) = (Uuid::new_v4(), Uuid::new_v4());
    assert!(reserve(id, target), "first sight of an id reserves it");
    assert!(!reserve(id, target), "second sight must not re-reserve");
    // The reservation IS a receipt: status is answerable from the moment of
    // reservation, before any delivery has happened.
    let receipt = status(id).expect("a reservation is status-checkable");
    assert_eq!(receipt.state, ReceiptState::Queued);
    assert_eq!(receipt.target, target);
}

#[test]
fn distinct_ids_reserve_independently() {
    // Guard against over-deduping: two notifies are two notifies, even to the
    // same target.
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    let target = Uuid::new_v4();
    assert!(reserve(a, target));
    assert!(reserve(b, target));
}

#[test]
fn forget_releases_a_reservation() {
    // #199: a real non-delivery verdict banked NOTHING, so its id must be
    // released — a retry carrying that id must stay free to deliver, and a
    // status poll must not claim a delivery that never happened.
    let (id, target) = (Uuid::new_v4(), Uuid::new_v4());
    assert!(reserve(id, target));
    forget(id);
    assert!(
        status(id).is_none(),
        "a released id leaves no receipt behind"
    );
    assert!(reserve(id, target), "a released id can be reserved afresh");
}

#[test]
fn a_delivered_id_stays_reserved_against_the_post_delivery_record() {
    // The delivery path re-records the receipt after a successful route;
    // that must not release the reservation, or the retry the reservation
    // exists to catch would deliver a second copy.
    let (id, target) = (Uuid::new_v4(), Uuid::new_v4());
    assert!(reserve(id, target));
    record_queued(id, target);
    assert!(!reserve(id, target));
}
