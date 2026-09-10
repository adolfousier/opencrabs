//! A scheduled job sends where it was configured to, or nowhere.
//!
//! A cron turn has no channel origin, so the proactive send path took its
//! destination from the tool input, and a job's turn picks that input up from
//! whatever it reads. On 2026-08-21 a memory note from two weeks earlier
//! carried a chat id and thread id under the heading "CONTINUE THIS TASK", and
//! a job posted its report into that group: one it was never configured for,
//! whose members had asked for nothing.

use crate::cron::send_scope::{
    PermittedTarget, SendPermission, may_send_to, permission, with_send_target,
};

const CONFIGURED: i64 = -1004252074515;
const SOMEWHERE_ELSE: i64 = -1004428873948;

#[tokio::test]
async fn outside_a_job_nothing_is_restricted() {
    // The rule exists to stop a job reaching chats it was never given, not to
    // police an ordinary reply in a chat the user is talking in.
    assert_eq!(permission(), SendPermission::Unscoped);
    assert!(may_send_to(SOMEWHERE_ELSE));
}

#[tokio::test]
async fn a_job_may_send_to_the_chat_it_was_given() {
    with_send_target(Some(CONFIGURED), async {
        assert_eq!(
            permission(),
            SendPermission::Permitted(vec![PermittedTarget {
                channel: "telegram",
                target_id: CONFIGURED.to_string(),
            }])
        );
        assert!(may_send_to(CONFIGURED));
    })
    .await;
}

#[tokio::test]
async fn a_job_may_not_send_to_any_other_chat() {
    // The exact leak: the destination came from a recalled memory, not from
    // the job's configuration.
    with_send_target(Some(CONFIGURED), async {
        assert!(
            !may_send_to(SOMEWHERE_ELSE),
            "a chat id found in memory is not permission to post there"
        );
    })
    .await;
}

#[tokio::test]
async fn a_job_without_a_target_sends_nowhere() {
    // Not the owner's DM, not a guess, nowhere. Its output stays in its session.
    with_send_target(None, async {
        assert_eq!(permission(), SendPermission::Nowhere);
        assert!(!may_send_to(CONFIGURED));
        assert!(!may_send_to(SOMEWHERE_ELSE));
    })
    .await;
}

#[tokio::test]
async fn the_scope_does_not_outlive_the_job() {
    // Task-local, so a sibling job on the scheduler is unaffected and the
    // restriction is gone once the turn ends.
    with_send_target(Some(CONFIGURED), async {
        assert!(!may_send_to(SOMEWHERE_ELSE));
    })
    .await;
    assert_eq!(permission(), SendPermission::Unscoped);
    assert!(may_send_to(SOMEWHERE_ELSE));
}

#[tokio::test]
async fn the_refusal_names_what_to_change() {
    with_send_target(Some(CONFIGURED), async {
        let msg = crate::cron::send_scope::refusal(SOMEWHERE_ELSE);
        assert!(msg.contains("deliver_to"), "got: {msg}");
        assert!(msg.contains(&CONFIGURED.to_string()), "got: {msg}");
    })
    .await;
}
