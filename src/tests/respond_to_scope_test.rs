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
