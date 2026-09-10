//! Integration tests for Step 5: send_scope authority extension + loud
//! fire-time failure pins (#148).
//!
//! Scope law: a cron turn may deliver ONLY to its configured target, and
//! targetless jobs send nowhere. This is now authority-aware across
//! telegram, discord, slack, and whatsapp.
//!
//! Fire-time pin: a leaked `oc://` target URL reaching the scheduler's
//! `deliver_result` fails loudly and records `delivery_failed` on the run
//! row — the scheduler refuses fire-time resolution.

use crate::cron::send_scope::{
    may_send, may_send_to, permission, refusal_for, with_permitted_targets, with_send_target,
    PermittedTarget, SendPermission,
};

#[test]
fn outside_cron_turn_is_unscoped() {
    assert_eq!(permission(), SendPermission::Unscoped);
    assert!(may_send("telegram", "-100123"));
    assert!(may_send("discord", "123456789"));
    assert!(may_send("slack", "C12345678"));
    assert!(may_send("whatsapp", "1234567890@s.whatsapp.net"));
    assert!(may_send_to(12345));
}

#[tokio::test]
async fn cron_with_no_targets_sends_nowhere_across_all_authorities() {
    with_permitted_targets(Some(Vec::new()), async {
        assert_eq!(permission(), SendPermission::Nowhere);
        assert!(!may_send("telegram", "-100123"));
        assert!(!may_send("discord", "123456789"));
        assert!(!may_send("slack", "C12345678"));
        assert!(!may_send("whatsapp", "1234567890@s.whatsapp.net"));
        assert!(!may_send_to(12345));

        let r = refusal_for("telegram", "-100123");
        assert!(r.contains("no deliver_to"), "{r}");
        let r = refusal_for("discord", "123");
        assert!(r.contains("no deliver_to"), "{r}");
    })
    .await;
}

#[tokio::test]
async fn telegram_authority_scope_pin() {
    let targets = vec![PermittedTarget {
        channel: "telegram",
        target_id: "-100123".into(),
    }];
    with_permitted_targets(Some(targets), async {
        assert!(may_send("telegram", "-100123"));
        assert!(!may_send("telegram", "-100999"));
        assert!(!may_send("discord", "123"));
        assert!(!may_send("slack", "C1"));
        assert!(!may_send("whatsapp", "123@s.whatsapp.net"));

        let r = refusal_for("telegram", "-100999");
        assert!(r.contains("may only send to [telegram:-100123]"), "{r}");
        assert!(r.contains("attempted telegram:-100999") || r.contains("targeted telegram:-100999"), "{r}");
    })
    .await;
}

#[tokio::test]
async fn discord_authority_scope_pin() {
    let targets = vec![PermittedTarget {
        channel: "discord",
        target_id: "888999111".into(),
    }];
    with_permitted_targets(Some(targets), async {
        assert!(may_send("discord", "888999111"));
        assert!(!may_send("discord", "999000111"));
        assert!(!may_send("telegram", "-100123"));

        let r = refusal_for("discord", "999000111");
        assert!(r.contains("may only send to [discord:888999111]"), "{r}");
    })
    .await;
}

#[tokio::test]
async fn slack_authority_scope_pin() {
    let targets = vec![PermittedTarget {
        channel: "slack",
        target_id: "C_DEV_CHANNEL".into(),
    }];
    with_permitted_targets(Some(targets), async {
        assert!(may_send("slack", "C_DEV_CHANNEL"));
        assert!(!may_send("slack", "C_OTHER"));
        assert!(!may_send("telegram", "-100123"));

        let r = refusal_for("slack", "C_OTHER");
        assert!(r.contains("may only send to [slack:C_DEV_CHANNEL]"), "{r}");
    })
    .await;
}

#[tokio::test]
async fn whatsapp_authority_scope_pin() {
    let targets = vec![PermittedTarget {
        channel: "whatsapp",
        target_id: "79991234567@s.whatsapp.net".into(),
    }];
    with_permitted_targets(Some(targets), async {
        assert!(may_send("whatsapp", "79991234567@s.whatsapp.net"));
        assert!(!may_send("whatsapp", "79999999999@s.whatsapp.net"));
        assert!(!may_send("telegram", "-100123"));

        let r = refusal_for("whatsapp", "79999999999@s.whatsapp.net");
        assert!(r.contains("may only send to [whatsapp:79991234567@s.whatsapp.net]"), "{r}");
    })
    .await;
}

#[tokio::test]
async fn multi_target_scope_pin() {
    let targets = vec![
        PermittedTarget {
            channel: "telegram",
            target_id: "-100111".into(),
        },
        PermittedTarget {
            channel: "slack",
            target_id: "C_CRON".into(),
        },
    ];
    with_permitted_targets(Some(targets), async {
        assert!(may_send("telegram", "-100111"));
        assert!(may_send("slack", "C_CRON"));
        assert!(!may_send("discord", "any"));
        assert!(!may_send("telegram", "-100222"));
    })
    .await;
}

#[tokio::test]
async fn legacy_with_send_target_compat_helper() {
    with_send_target(Some(-100777), async {
        assert!(may_send_to(-100777));
        assert!(!may_send_to(-100888));
        assert!(may_send("telegram", "-100777"));
        assert!(!may_send("discord", "1"));
    })
    .await;

    with_send_target(None, async {
        assert_eq!(permission(), SendPermission::Nowhere);
        assert!(!may_send_to(-100777));
    })
    .await;
}
