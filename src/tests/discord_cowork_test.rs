//! `/cowork` on Discord (#2015): the owner opens a channel or thread, and the
//! write records `open = true` plus the channel name. Members are not
//! registered; `open` admits them.

use crate::channels::discord::commands::with_menu_builtins;
use crate::channels::discord::cowork::{cowork_discord_channel, is_cowork_command};
use std::cell::RefCell;

#[test]
fn cowork_recognised_only_as_the_whole_command() {
    assert!(is_cowork_command("/cowork"));
    assert!(is_cowork_command("  /cowork  "));
    assert!(!is_cowork_command("/coworker"));
    assert!(!is_cowork_command("please /cowork"));
    assert!(!is_cowork_command("/respond_to all"));
}

#[test]
fn owner_in_a_server_channel_opens_it_with_its_name() {
    let written = RefCell::new(Vec::new());
    let reply = cowork_discord_channel(
        "/cowork",
        true,
        true,
        "4242",
        Some("general"),
        |id, name| {
            written
                .borrow_mut()
                .push((id.to_string(), name.map(str::to_string)));
            Ok(())
        },
    )
    .expect("recognised");
    assert!(reply.contains("This channel is open"), "{reply}");
    assert!(
        reply.contains("threads and forum posts included"),
        "{reply}"
    );
    assert!(reply.contains("Members are not registered"), "{reply}");
    assert_eq!(
        written.into_inner(),
        vec![("4242".to_string(), Some("general".to_string()))]
    );
}

#[test]
fn a_failed_lookup_still_opens_the_channel_without_a_name() {
    let written = RefCell::new(Vec::new());
    cowork_discord_channel("/cowork", true, true, "7", None, |id, name| {
        written
            .borrow_mut()
            .push((id.to_string(), name.map(str::to_string)));
        Ok(())
    });
    assert_eq!(written.into_inner(), vec![("7".to_string(), None)]);
}

#[test]
fn non_owner_is_refused_and_nothing_is_written() {
    let reply = cowork_discord_channel("/cowork", false, true, "1", Some("x"), |_, _| {
        panic!("a refused command must not write")
    })
    .expect("recognised");
    assert!(reply.contains("restricted to the bot owner"), "{reply}");
}

#[test]
fn dm_is_refused_because_it_is_not_a_server_channel() {
    let reply = cowork_discord_channel("/cowork", true, false, "1", None, |_, _| {
        panic!("a DM must not write")
    })
    .expect("recognised");
    assert!(reply.contains("opens a server channel"), "{reply}");
}

#[test]
fn write_failure_is_reported() {
    let reply = cowork_discord_channel("/cowork", true, true, "1", Some("x"), |_, _| {
        Err("disk full".to_string())
    })
    .expect("recognised");
    assert!(reply.contains("Could not open this channel"), "{reply}");
    assert!(reply.contains("disk full"), "{reply}");
}

#[test]
fn other_text_is_not_cowork() {
    assert_eq!(
        cowork_discord_channel("/respond_to", true, true, "1", None, |_, _| panic!("no")),
        None
    );
}

#[test]
fn discord_menu_offers_cowork_alongside_respond_to() {
    let menu = with_menu_builtins(Vec::new());
    for name in ["/respond_to", "/cowork"] {
        assert_eq!(menu.iter().filter(|c| c.name == name).count(), 1, "{name}");
    }
}
