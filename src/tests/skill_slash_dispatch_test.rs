//! Tests for the skill fallback in the channel slash-command dispatcher.
//!
//! When a user types `/<name>` in a channel, the resolver tries (in order):
//! built-in channel commands (`/help`, `/usage`, …) handled before this code
//! path, then explicit user-defined commands from `commands.toml`, then
//! auto-registered skills (`SKILL.md` slug match). Unknown commands return
//! `NotACommand`. These tests cover the skill leg and the ordering between
//! user commands and skills.

use crate::brain::commands::UserCommand;
use crate::brain::skills::{Skill, SkillSource};
use crate::channels::commands::{ChannelCommand, match_user_command_inner};

fn skill(name: &str, body: &str) -> Skill {
    Skill {
        name: name.to_string(),
        slash_name: format!("/{name}"),
        description: format!("test skill {name}"),
        body: body.to_string(),
        globs: Vec::new(),
        review_gate: false,
        source: SkillSource::Builtin,
    }
}

fn user_cmd(name: &str, prompt: &str) -> UserCommand {
    UserCommand {
        name: name.to_string(),
        description: String::new(),
        action: "prompt".to_string(),
        prompt: prompt.to_string(),
    }
}

#[test]
fn skill_slash_dispatches_skill_body_as_prompt() {
    let skills = vec![skill("security-audit", "Run a comprehensive audit.")];
    match match_user_command_inner("/security-audit", &[], &skills) {
        ChannelCommand::UserPrompt(p) => assert_eq!(p, "Run a comprehensive audit."),
        _ => panic!("expected UserPrompt for skill dispatch"),
    }
}

#[test]
fn skill_args_append_after_blank_line() {
    // Args after the slash get appended so callers can pass extra context
    // without writing a custom commands.toml wrapper.
    let skills = vec![skill("audit", "Body of the audit skill.")];
    match match_user_command_inner("/audit focus on auth code", &[], &skills) {
        ChannelCommand::UserPrompt(p) => {
            assert_eq!(p, "Body of the audit skill.\n\nfocus on auth code");
        }
        _ => panic!("expected UserPrompt with appended args"),
    }
}

#[test]
fn user_command_wins_over_same_named_skill() {
    // Same slug present in both — user's commands.toml entry is the
    // explicit override and must take precedence.
    let cmds = vec![user_cmd("/audit", "User's own audit prompt.")];
    let skills = vec![skill("audit", "Built-in skill body.")];
    match match_user_command_inner("/audit", &cmds, &skills) {
        ChannelCommand::UserPrompt(p) => assert_eq!(p, "User's own audit prompt."),
        _ => panic!("expected user command to win"),
    }
}

#[test]
fn skill_only_dispatches_when_no_user_command_matches() {
    let cmds = vec![user_cmd("/other", "irrelevant")];
    let skills = vec![skill("audit", "Skill body.")];
    match match_user_command_inner("/audit", &cmds, &skills) {
        ChannelCommand::UserPrompt(p) => assert_eq!(p, "Skill body."),
        _ => panic!("expected skill fallback to fire"),
    }
}

#[test]
fn unknown_slash_returns_unknown_command_even_with_skills_loaded() {
    let skills = vec![skill("audit", "Skill body.")];
    match match_user_command_inner("/does-not-exist", &[], &skills) {
        ChannelCommand::UnknownCommand(msg) => assert!(msg.contains("/does-not-exist")),
        _other => panic!("expected UnknownCommand, got something else"),
    }
}

#[test]
fn skill_with_no_args_returns_body_unchanged() {
    let skills = vec![skill("estimate", "Estimate the cost.")];
    match match_user_command_inner("/estimate", &[], &skills) {
        ChannelCommand::UserPrompt(p) => assert_eq!(p, "Estimate the cost."),
        _ => panic!("expected UserPrompt with body verbatim"),
    }
}

#[test]
fn review_gated_skill_dispatch_prepends_reminder() {
    // `review_gate: true` skills carry the gate reminder through the slash
    // dispatch; unflagged skills (above) are byte-identical to before.
    let gated = Skill {
        name: "drop-release".to_string(),
        slash_name: "/drop-release".to_string(),
        description: "gated release flow".to_string(),
        body: "Draft the release.".to_string(),
        globs: Vec::new(),
        review_gate: true,
        source: crate::brain::skills::SkillSource::User,
    };
    let skills = vec![gated];
    match match_user_command_inner("/drop-release", &[], &skills) {
        ChannelCommand::UserPrompt(p) => {
            assert!(p.contains("SKILL REVIEW GATE"), "reminder missing: {p}");
            assert!(p.contains("Draft the release."), "body missing: {p}");
        }
        _ => panic!("expected UserPrompt for gated skill dispatch"),
    }
}

#[test]
fn skill_dispatch_ignores_leading_trailing_arg_whitespace() {
    let skills = vec![skill("audit", "Body.")];
    match match_user_command_inner("/audit    extra args   ", &[], &skills) {
        ChannelCommand::UserPrompt(p) => {
            // First space is the separator; remaining whitespace inside
            // args is preserved verbatim, but trailing whitespace is
            // stripped because the splitter trims the args side.
            assert_eq!(p, "Body.\n\nextra args");
        }
        _ => panic!("expected UserPrompt"),
    }
}
