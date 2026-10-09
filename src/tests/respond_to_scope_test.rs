//! `/respond_to` outside Telegram (#2013): the surfaces without per-channel
//! config answer from their own channel-level setting and never write, and the
//! Discord `/` menu offers the command.

use crate::brain::UserCommand;
use crate::channels::discord::commands::with_menu_builtins;
use crate::channels::respond_to_scope::respond_to_outside_telegram;
use crate::config::RespondTo;

#[test]
fn other_text_is_not_the_command() {
    for text in [
        "hello",
        "/respond_tomato",
        "/respond_to_x",
        "please /respond_to all",
    ] {
        assert_eq!(
            respond_to_outside_telegram(text, true, "Discord", Some(&RespondTo::All)),
            None,
            "{text:?}"
        );
    }
}

#[test]
fn non_owner_is_refused_on_every_surface() {
    for platform in ["Discord", "Slack", "WhatsApp"] {
        let reply = respond_to_outside_telegram("/respond_to all", false, platform, None)
            .expect("the command is recognised");
        assert!(
            reply.contains("restricted to the bot owner"),
            "{platform}: {reply}"
        );
    }
}

#[test]
fn owner_bare_command_shows_the_channel_mode_and_points_at_config() {
    let reply =
        respond_to_outside_telegram("/respond_to", true, "Discord", Some(&RespondTo::Mention))
            .expect("recognised");
    assert!(reply.contains("**mention**"), "{reply}");
    assert!(reply.contains("[channels.discord] respond_to"), "{reply}");
    assert!(reply.contains("#2014"), "{reply}");
}

#[test]
fn owner_with_a_mode_is_told_nothing_was_written() {
    for arg in ["all", "mention", "auto", "dm_only", "bogus"] {
        let text = format!("/respond_to {arg}");
        let reply = respond_to_outside_telegram(&text, true, "Slack", Some(&RespondTo::Mention))
            .expect("recognised");
        assert!(reply.contains("does not change Slack"), "{arg}: {reply}");
        assert!(reply.contains("Nothing was written"), "{arg}: {reply}");
        assert!(
            reply.contains("[channels.slack] respond_to"),
            "{arg}: {reply}"
        );
    }
}

#[test]
fn whatsapp_has_no_respond_mode_and_names_its_setting() {
    let reply =
        respond_to_outside_telegram("/respond_to", true, "WhatsApp", None).expect("recognised");
    assert!(reply.contains("response_policy"), "{reply}");
    assert!(reply.contains("[channels.whatsapp]"), "{reply}");
}

#[test]
fn discord_menu_offers_respond_to_once() {
    let menu = with_menu_builtins(Vec::new());
    assert_eq!(menu.iter().filter(|c| c.name == "/respond_to").count(), 1);

    let user_defined = UserCommand {
        name: "/respond_to".to_string(),
        description: "mine".to_string(),
        action: "system".to_string(),
        prompt: String::new(),
    };
    let kept = with_menu_builtins(vec![user_defined]);
    assert_eq!(
        kept.len(),
        1,
        "a user command of the same name is not duplicated"
    );
    assert_eq!(kept[0].description, "mine");
}

// ── Discord channel scope (#2014) ───────────────────────────────────────

use crate::channels::respond_to_scope::respond_to_discord_channel;
use std::cell::RefCell;

fn no_write(_: &str, _: &str) -> Result<(), String> {
    panic!("a bare or refused command must not write")
}

#[test]
fn discord_bare_command_shows_this_channels_effective_mode() {
    let reply = respond_to_discord_channel("/respond_to", true, "4242", &RespondTo::All, no_write)
        .expect("recognised");
    assert!(reply.contains("**all**"), "{reply}");
    assert!(
        reply.contains("[channels.discord.channels.4242] respond_to"),
        "{reply}"
    );
}

#[test]
fn discord_mode_writes_that_channels_own_entry() {
    let written = RefCell::new(Vec::new());
    let reply = respond_to_discord_channel(
        "/respond_to mention",
        true,
        "4242",
        &RespondTo::All,
        |cid, mode| {
            written
                .borrow_mut()
                .push((cid.to_string(), mode.to_string()));
            Ok(())
        },
    )
    .expect("recognised");
    assert!(reply.contains("switched to **mention**"), "{reply}");
    assert_eq!(
        written.into_inner(),
        vec![("4242".to_string(), "mention".to_string())]
    );
}

#[test]
fn discord_accepts_dm_only_and_the_documented_spellings() {
    for (arg, label) in [
        ("all", "all"),
        ("mentions", "mention"),
        ("auto", "auto"),
        ("dm_only", "dm_only"),
    ] {
        let text = format!("/respond_to {arg}");
        let captured = RefCell::new(String::new());
        respond_to_discord_channel(&text, true, "1", &RespondTo::Mention, |_, mode| {
            *captured.borrow_mut() = mode.to_string();
            Ok(())
        });
        assert_eq!(captured.into_inner(), label, "{arg}");
    }
}

#[test]
fn discord_unknown_mode_is_refused_without_a_write() {
    let reply =
        respond_to_discord_channel("/respond_to loud", true, "1", &RespondTo::Mention, no_write)
            .expect("recognised");
    assert!(reply.contains("Unknown mode"), "{reply}");
}

#[test]
fn discord_write_failure_is_reported() {
    let reply =
        respond_to_discord_channel("/respond_to all", true, "1", &RespondTo::Mention, |_, _| {
            Err("disk full".to_string())
        })
        .expect("recognised");
    assert!(reply.contains("Could not save"), "{reply}");
    assert!(reply.contains("disk full"), "{reply}");
}

#[test]
fn discord_non_owner_is_refused_and_nothing_is_written() {
    let reply =
        respond_to_discord_channel("/respond_to all", false, "1", &RespondTo::Mention, no_write)
            .expect("recognised");
    assert!(reply.contains("restricted to the bot owner"), "{reply}");
}

#[test]
fn respond_to_command_is_recognised_for_the_mention_gate() {
    use crate::channels::respond_to_scope::is_respond_to_command;
    assert!(is_respond_to_command("/respond_to"));
    assert!(is_respond_to_command("/respond_to all"));
    assert!(is_respond_to_command("  /respond_to mention  "));
    assert!(!is_respond_to_command("/respond_tomato"));
    assert!(!is_respond_to_command("please /respond_to all"));
    assert!(!is_respond_to_command("/models"));
}
