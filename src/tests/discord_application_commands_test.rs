//! Application command projection for Discord (#1850).
//!
//! Every rule in `channels::discord::commands` that Discord enforces with a 400
//! is tested here against the pure planner, because there is no Discord
//! application in CI: the builders are inspected through their serialized wire
//! surface, which is exactly what the HTTP layer would send.

use crate::brain::UserCommand;
use crate::channels::discord::commands::{
    ARGS_OPTION, ARGS_OPTION_DESCRIPTION, DESCRIPTION_MAX, DropReason, Dropped, GUILD_COMMAND_CAP,
    NAME_MAX, TREE_CHAR_CAP, description_for, holds_allowed_role, identity_admitted, plan_commands,
    sanitize_name, sync_key,
};
use serenity::model::id::GuildId;

fn cmd(name: &str, description: &str) -> UserCommand {
    UserCommand {
        name: name.to_string(),
        description: description.to_string(),
        action: "prompt".to_string(),
        prompt: format!("prompt for {name}"),
    }
}

fn names(plan: &crate::channels::discord::commands::CommandPlan) -> Vec<String> {
    plan.entries.iter().map(|(n, _, _)| n.clone()).collect()
}

#[test]
fn leading_slash_is_stripped_and_names_are_lowercased() {
    assert_eq!(sanitize_name("/deploy").as_deref(), Some("deploy"));
    assert_eq!(sanitize_name("Deploy").as_deref(), Some("deploy"));
    assert_eq!(sanitize_name("  /Deploy  ").as_deref(), Some("deploy"));
}

#[test]
fn illegal_characters_become_dashes_not_rejections() {
    // Discord's grammar is `^[\w-]{1,32}$`. The catalog is free-form, so a name
    // like "fix CI now" must survive as "fix-ci-now" rather than lose the
    // command entirely.
    assert_eq!(sanitize_name("fix CI now").as_deref(), Some("fix-ci-now"));
    assert_eq!(sanitize_name("a.b:c").as_deref(), Some("a-b-c"));
    assert_eq!(sanitize_name("run!").as_deref(), Some("run"));
    // Underscores and digits are legal and must not be touched.
    assert_eq!(sanitize_name("/next_2").as_deref(), Some("next_2"));
}

#[test]
fn edge_dashes_are_trimmed_so_nothing_starts_or_ends_on_one() {
    assert_eq!(sanitize_name("!!bang!!").as_deref(), Some("bang"));
    assert_eq!(sanitize_name("///").as_deref(), None);
    assert_eq!(sanitize_name("!!!").as_deref(), None);
    assert_eq!(sanitize_name("").as_deref(), None);
}

#[test]
fn names_are_clamped_to_thirty_two_characters() {
    let long = format!("/{}", "x".repeat(60));
    let clamped = sanitize_name(&long).expect("clamped name");
    assert_eq!(clamped.chars().count(), NAME_MAX);
    assert!(clamped.chars().all(|c| c == 'x'));
}

#[test]
fn descriptions_are_clamped_to_one_hundred_characters() {
    let long = "d".repeat(400);
    let clamped = description_for(&long, "some-cmd");
    assert_eq!(clamped.chars().count(), DESCRIPTION_MAX);
}

#[test]
fn empty_description_falls_back_to_the_name() {
    // Discord requires a non-empty description for CHAT_INPUT, and the catalog
    // allows omitting it.
    assert_eq!(description_for("", "some-cmd"), "some-cmd");
    assert_eq!(description_for("   ", "some-cmd"), "some-cmd");
    assert_eq!(description_for("  real text  ", "some-cmd"), "real text");
}

#[test]
fn every_command_carries_exactly_one_optional_string_args_option() {
    let plan = plan_commands(&[cmd("/btw", "spawn a side agent")]);
    assert_eq!(plan.commands.len(), 1);
    assert!(plan.dropped.is_empty(), "{:?}", plan.dropped);

    let wire = serde_json::to_value(&plan.commands[0]).expect("serialize builder");
    assert_eq!(wire["name"], "btw");
    assert_eq!(wire["description"], "spawn a side agent");
    assert_eq!(wire["type"], 1, "CHAT_INPUT");

    let options = wire["options"].as_array().expect("options array");
    assert_eq!(options.len(), 1, "one option, no sub-commands: {options:?}");
    assert_eq!(options[0]["name"], ARGS_OPTION);
    assert_eq!(options[0]["description"], ARGS_OPTION_DESCRIPTION);
    assert_eq!(options[0]["type"], 3, "STRING");
    assert_eq!(
        options[0]["required"], false,
        "args must be optional or /help with no argument becomes invalid input"
    );
}

#[test]
fn collisions_drop_the_later_entry_and_name_the_winner() {
    // `/foo bar` and `/foo-bar` are the same Discord command. Silently merging
    // them would run one prompt for two names the user believes are distinct.
    let plan = plan_commands(&[cmd("/foo-bar", "first"), cmd("/foo bar", "second")]);
    assert_eq!(names(&plan), vec!["foo-bar".to_string()]);
    assert_eq!(
        plan.dropped,
        vec![Dropped {
            source: "/foo bar".to_string(),
            reason: DropReason::Collision {
                kept: "/foo-bar".to_string()
            },
        }]
    );
}

#[test]
fn unsanitizable_names_are_dropped_loudly_and_the_rest_survive() {
    // One broken entry must not take the whole guild's menu with it.
    let plan = plan_commands(&[cmd("///", "broken"), cmd("/ok", "fine")]);
    assert_eq!(names(&plan), vec!["ok".to_string()]);
    assert_eq!(plan.commands.len(), 1);
    assert_eq!(
        plan.dropped,
        vec![Dropped {
            source: "///".to_string(),
            reason: DropReason::InvalidName,
        }]
    );
}

#[test]
fn past_the_guild_cap_the_head_of_the_file_wins_and_the_tail_is_reported() {
    // Same ordering rule as Telegram's `trim_catalog_to_budget`, so both
    // channels drop the same entries for the same reason.
    let catalog: Vec<UserCommand> = (0..GUILD_COMMAND_CAP + 3)
        .map(|i| cmd(&format!("/cmd{i:03}"), &format!("description {i}")))
        .collect();
    let plan = plan_commands(&catalog);

    assert_eq!(plan.commands.len(), GUILD_COMMAND_CAP);
    assert_eq!(
        names(&plan).first().map(String::as_str),
        Some("cmd000"),
        "catalog order decides who is kept"
    );
    assert_eq!(
        names(&plan).last().map(String::as_str),
        Some("cmd099"),
        "catalog order decides who is kept"
    );
    let dropped: Vec<&str> = plan.dropped.iter().map(|d| d.source.as_str()).collect();
    assert_eq!(dropped, vec!["/cmd100", "/cmd101", "/cmd102"]);
    assert!(
        plan.dropped.iter().all(|d| d.reason == DropReason::OverCap),
        "{:?}",
        plan.dropped
    );
}

#[test]
fn a_tree_over_eight_thousand_characters_is_refused_not_truncated() {
    // Discord counts the whole tree; a 400-char description is clamped to 100,
    // so the budget only bites with many commands. Build enough of them that
    // the cap is the constraint, not the per-command cap.
    let each_cost = NAME_MAX + DESCRIPTION_MAX + ARGS_OPTION.len() + ARGS_OPTION_DESCRIPTION.len();
    let needed = TREE_CHAR_CAP / each_cost + 2;
    // Names are padded to the full NAME_MAX so the cost model above matches
    // what the planner charges. A short name costs 135 chars instead of 162,
    // 51 of them fit the 8000 budget, and the cap legitimately never bites.
    let catalog: Vec<UserCommand> = (0..needed)
        .map(|i| {
            let mut name = format!("tree{i:04}");
            name.push_str(&"x".repeat(NAME_MAX - name.len()));
            cmd(&format!("/{name}"), &"d".repeat(400))
        })
        .collect();
    let plan = plan_commands(&catalog);

    assert!(
        plan.commands.len() < needed,
        "the tree cap must bite before {} commands",
        needed
    );
    assert!(
        plan.dropped
            .iter()
            .any(|d| d.reason == DropReason::TreeTooLarge),
        "a command that blows the tree budget must be reported, got {:?}",
        plan.dropped
    );
    let kept_chars: usize = plan
        .entries
        .iter()
        .map(|(n, _, d)| n.chars().count() + d.chars().count())
        .sum::<usize>()
        + plan.entries.len() * (ARGS_OPTION.len() + ARGS_OPTION_DESCRIPTION.len());
    assert!(
        kept_chars <= TREE_CHAR_CAP,
        "registered tree is {kept_chars} chars, over the {TREE_CHAR_CAP} cap"
    );
}

#[test]
fn signature_is_stable_across_rereads_and_reordering() {
    let a = plan_commands(&[cmd("/one", "first"), cmd("/two", "second")]);
    let b = plan_commands(&[cmd("/one", "first"), cmd("/two", "second")]);
    let reordered = plan_commands(&[cmd("/two", "second"), cmd("/one", "first")]);
    assert_eq!(a.signature(), b.signature(), "an unchanged reread");
    assert_eq!(
        a.signature(),
        reordered.signature(),
        "catalog order must not force a re-sync"
    );
}

#[test]
fn signature_moves_when_a_registered_description_moves() {
    let before = plan_commands(&[cmd("/one", "first")]);
    let after = plan_commands(&[cmd("/one", "changed")]);
    assert_ne!(
        before.signature(),
        after.signature(),
        "a description the client displays is part of the set"
    );
}

#[test]
fn signature_moves_when_a_command_is_added_or_dropped() {
    let one = plan_commands(&[cmd("/one", "first")]);
    let two = plan_commands(&[cmd("/one", "first"), cmd("/two", "second")]);
    assert_ne!(one.signature(), two.signature());

    // A dropped entry that never registered must not change the signature
    // either way: the wire set is identical before and after it.
    let with_broken = plan_commands(&[cmd("/one", "first"), cmd("///", "broken")]);
    assert_eq!(one.signature(), with_broken.signature());
}

#[test]
fn prompt_text_is_never_part_of_the_wire_surface() {
    // The prompt lives in the catalog and is read by the tool on dispatch;
    // shipping it as a description would leak command internals into a field
    // every guild member can see in the client.
    let secret = "the private prompt body";
    let plan = plan_commands(&[UserCommand {
        name: "/secret".to_string(),
        description: "public blurb".to_string(),
        action: "prompt".to_string(),
        prompt: secret.to_string(),
    }]);
    let wire = serde_json::to_string(&plan.commands).expect("serialize");
    assert!(
        !wire.contains(secret),
        "prompt body leaked onto the wire: {wire}"
    );
    assert!(wire.contains("public blurb"));
}

#[test]
fn the_sync_key_is_stable_across_reconnects_with_an_unchanged_setup() {
    // The retry loop rebuilds the client on every gateway drop. A reconnect of
    // the same catalog into the same guilds must cost no API call, so the key
    // the second pass computes has to equal the one stored by the first.
    let catalog = [cmd("/one", "first"), cmd("/two", "second")];
    let sig = plan_commands(&catalog).signature();
    let guilds = vec![GuildId::from(11u64), GuildId::from(22u64)];
    assert_eq!(sync_key(sig, &guilds), sync_key(sig, &guilds));
}

#[test]
fn the_sync_key_ignores_the_order_the_gateway_reported_guilds_in() {
    let catalog = [cmd("/one", "first")];
    let sig = plan_commands(&catalog).signature();
    let a = vec![GuildId::from(11u64), GuildId::from(22u64)];
    let b = vec![GuildId::from(22u64), GuildId::from(11u64)];
    assert_eq!(sync_key(sig, &a), sync_key(sig, &b));
}

#[test]
fn joining_a_guild_moves_the_key_without_touching_the_catalog() {
    // This is the reason membership is part of the key and not just the
    // command set: a server added while the process is up has to be found
    // without waiting for someone to edit `commands.toml`.
    let catalog = [cmd("/one", "first")];
    let sig = plan_commands(&catalog).signature();
    let before = vec![GuildId::from(11u64)];
    let after = vec![GuildId::from(11u64), GuildId::from(22u64)];
    assert_ne!(sync_key(sig, &before), sync_key(sig, &after));
}

#[test]
fn editing_the_catalog_moves_the_key_for_the_same_guilds() {
    let guilds = vec![GuildId::from(11u64)];
    let before = plan_commands(&[cmd("/one", "first")]).signature();
    let after = plan_commands(&[cmd("/one", "renamed description")]).signature();
    assert_ne!(sync_key(before, &guilds), sync_key(after, &guilds));
}

fn roles(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// An empty allowlist, no roles and no owner is UNCONFIGURED, and unconfigured
/// denies everybody. The old reading ("empty means everyone") turned a bot that
/// was still being configured into a public endpoint, and a slash command is
/// offered to every member of the guild, so this is the branch that matters.
#[test]
fn an_unconfigured_channel_admits_nobody_not_everybody() {
    assert!(
        !identity_admitted(true, true, true, true),
        "even a set that looks admitted must deny while unconfigured"
    );
}

#[test]
fn a_configured_channel_admits_owner_allowlist_or_role() {
    assert!(identity_admitted(false, true, false, false), "owner");
    assert!(
        identity_admitted(false, false, true, false),
        "allowlisted id"
    );
    assert!(
        identity_admitted(false, false, false, true),
        "holder of an allowed role"
    );
    assert!(
        !identity_admitted(false, false, false, false),
        "a stranger in a configured guild is denied"
    );
}

/// Roles are compared as strings because ids cross a JSON boundary that way.
/// The message path does the same, and a mismatch here would silently deny a
/// user who can see the command in their client.
#[test]
fn allowed_roles_match_on_the_string_form_of_the_id() {
    assert!(holds_allowed_role(&roles(&["4242"]), &[4242]));
    assert!(!holds_allowed_role(&roles(&["4242"]), &[9999]));
    assert!(
        !holds_allowed_role(&roles(&[]), &[4242]),
        "no allowlisted role grants nothing, which is why a DM passing an \
         empty role list cannot walk past a guild-only allowlist"
    );
}
