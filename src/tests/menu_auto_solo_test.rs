//! #1155 & #248 — group auto-registration: the pure decision core.
//!
//! `evaluate_solo_group` decides whether an unconfigured group gets the full
//! owner catalog automatically under ChatMember scope. If the owner is present
//! in the observed member list, returns `SoloEval::Eligible`. Strangers and
//! extra bots do not block the owner from receiving their menu.

use crate::channels::telegram::menu_auto::{MemberView, SoloEval, evaluate_solo_group};

fn human(uid: i64) -> MemberView {
    MemberView {
        user_id: uid,
        is_bot: false,
    }
}

fn bot(uid: i64) -> MemberView {
    MemberView {
        user_id: uid,
        is_bot: true,
    }
}

const OWNER: i64 = 111;

#[test]
fn owner_plus_bots_is_eligible() {
    let members = vec![bot(1), bot(2), bot(3), human(OWNER), bot(4)];
    assert_eq!(evaluate_solo_group(&members, OWNER), SoloEval::Eligible);
}

#[test]
fn owner_with_other_humans_is_eligible() {
    let members = vec![human(OWNER), bot(7), human(222)];
    assert_eq!(evaluate_solo_group(&members, OWNER), SoloEval::Eligible);
}

#[test]
fn owner_with_multiple_humans_and_bots_is_eligible() {
    let members = vec![human(OWNER), bot(1), human(222), human(333), bot(2)];
    assert_eq!(evaluate_solo_group(&members, OWNER), SoloEval::Eligible);
}

#[test]
fn owner_absent_is_not_eligible() {
    // A group full of bots and strangers but no owner: registering an owner
    // scope would fail at the API and must not be attempted.
    let members = vec![bot(1), human(999)];
    assert_eq!(evaluate_solo_group(&members, OWNER), SoloEval::OwnerAbsent);
}

#[test]
fn empty_member_list_is_owner_absent() {
    // get_chat_administrators returned nothing usable: fail closed.
    assert_eq!(evaluate_solo_group(&[], OWNER), SoloEval::OwnerAbsent);
}

#[test]
fn owner_alone_is_eligible() {
    let members = vec![human(OWNER)];
    assert_eq!(evaluate_solo_group(&members, OWNER), SoloEval::Eligible);
}
