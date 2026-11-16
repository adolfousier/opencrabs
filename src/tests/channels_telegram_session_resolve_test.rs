use crate::channels::telegram::session_resolve::*;
use crate::db::models::TitleSource;

#[test]
fn dm_template_format() {
    let t = build_session_title(true, "Alice", 123, "", 456, None, None);
    assert_eq!(t, "Telegram: DM Alice (123) [chat:456]");
}

#[test]
fn should_not_clobber_auto_titled_dm() {
    let auto = "Telegram: Fix deploy [chat:133526395]";
    let template = build_session_title(true, "Alexey", 133526395, "", 133526395, None, None);
    assert!(!should_refresh_label(auto, &template, TitleSource::Auto));
}

#[test]
fn should_refresh_group_rename() {
    let old = "Telegram: Old Group [chat:-1]";
    let new = "Telegram: New Group [chat:-1]";
    assert!(should_refresh_label(old, new, TitleSource::Template));
}

#[test]
fn default_dm_still_refreshes_on_name_change() {
    let old = build_session_title(true, "Alice", 1, "", 99, None, None);
    let new = build_session_title(true, "Bob", 1, "", 99, None, None);
    assert!(should_refresh_label(&old, &new, TitleSource::Template));
}

#[test]
fn chat_bound_wins_over_suffix_candidate() {
    let a = uuid::Uuid::new_v4();
    let b = uuid::Uuid::new_v4();
    assert_eq!(
        choose_resolve_source(Some(a), false, Some(b)),
        ResolveSource::ChatBound
    );
}

#[test]
fn archived_bound_falls_through_to_suffix() {
    let a = uuid::Uuid::new_v4();
    let b = uuid::Uuid::new_v4();
    assert_eq!(
        choose_resolve_source(Some(a), true, Some(b)),
        ResolveSource::Suffix
    );
}

#[test]
fn session_idle_expired_within_and_past_window() {
    let recent = chrono::Utc::now() - chrono::Duration::minutes(30);
    assert!(!session_idle_expired(recent, Some(1.0)));

    let stale = chrono::Utc::now() - chrono::Duration::hours(2);
    assert!(session_idle_expired(stale, Some(1.0)));
    assert!(!session_idle_expired(stale, None));
}

#[test]
fn session_idle_expired_boundary_not_yet_expired() {
    let at_limit = chrono::Utc::now() - chrono::Duration::seconds(3600);
    assert!(!session_idle_expired(at_limit, Some(1.0)));
    let past_limit = chrono::Utc::now() - chrono::Duration::seconds(3601);
    assert!(session_idle_expired(past_limit, Some(1.0)));
}
