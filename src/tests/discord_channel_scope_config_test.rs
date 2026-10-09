//! Per-channel Discord settings (#2014): the resolution rules that the
//! message filter, the command gate and the ACL all read.

use crate::config::{DiscordChannelConfig, DiscordConfig, RespondTo};
use std::collections::HashMap;

fn cfg_with(channels: Vec<(&str, DiscordChannelConfig)>) -> DiscordConfig {
    DiscordConfig {
        respond_to: RespondTo::Mention,
        channels: channels
            .into_iter()
            .map(|(id, c)| (id.to_string(), c))
            .collect::<HashMap<_, _>>(),
        ..DiscordConfig::default()
    }
}

#[test]
fn no_entry_inherits_the_global_mode_and_stays_closed() {
    let cfg = cfg_with(vec![]);
    assert_eq!(cfg.respond_to_for("100", None), RespondTo::Mention);
    assert!(!cfg.channel_open("100", None));
}

#[test]
fn channel_mode_overrides_global() {
    let cfg = cfg_with(vec![(
        "100",
        DiscordChannelConfig {
            respond_to: Some(RespondTo::All),
            ..Default::default()
        },
    )]);
    assert_eq!(cfg.respond_to_for("100", None), RespondTo::All);
    assert_eq!(cfg.respond_to_for("200", None), RespondTo::Mention);
}

#[test]
fn thread_inherits_its_parent_mode_and_open() {
    let cfg = cfg_with(vec![(
        "100",
        DiscordChannelConfig {
            respond_to: Some(RespondTo::All),
            open: true,
            ..Default::default()
        },
    )]);
    assert_eq!(cfg.respond_to_for("555", Some("100")), RespondTo::All);
    assert!(cfg.channel_open("555", Some("100")));
}

#[test]
fn thread_setting_wins_over_its_parent() {
    let cfg = cfg_with(vec![
        (
            "100",
            DiscordChannelConfig {
                respond_to: Some(RespondTo::All),
                ..Default::default()
            },
        ),
        (
            "555",
            DiscordChannelConfig {
                respond_to: Some(RespondTo::DmOnly),
                ..Default::default()
            },
        ),
    ]);
    assert_eq!(cfg.respond_to_for("555", Some("100")), RespondTo::DmOnly);
}

#[test]
fn open_is_channel_scoped_and_does_not_leak_to_siblings() {
    let cfg = cfg_with(vec![(
        "100",
        DiscordChannelConfig {
            open: true,
            ..Default::default()
        },
    )]);
    assert!(cfg.channel_open("100", None));
    assert!(!cfg.channel_open("200", None));
    assert!(!cfg.channel_open("200", Some("300")));
}

#[test]
fn a_thread_that_is_its_own_parent_does_not_loop() {
    let cfg = cfg_with(vec![]);
    assert_eq!(cfg.respond_to_for("100", Some("100")), RespondTo::Mention);
}

#[test]
fn channels_section_parses_from_toml() {
    let toml_src = r#"
        respond_to = "mention"

        [channels."100"]
        name = "general"
        respond_to = "all"
        open = true
    "#;
    let cfg: DiscordConfig = toml::from_str(toml_src).expect("parses");
    let entry = cfg.channels.get("100").expect("entry");
    assert_eq!(entry.name.as_deref(), Some("general"));
    assert_eq!(entry.respond_to, Some(RespondTo::All));
    assert!(entry.open);
    assert_eq!(cfg.respond_to_for("100", None), RespondTo::All);
}
