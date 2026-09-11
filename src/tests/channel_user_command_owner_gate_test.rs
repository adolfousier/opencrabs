//! Security: user-defined commands and skills are owner-only on channels
//! (#975).
//!
//! Every built-in slash command was individually owner-gated, but the
//! catch-all arm that reaches `commands.toml` entries and skill slugs was not.
//! All four channel handlers then fall through to the agent with the matched
//! body as the prompt and no re-gate, so any allowlisted user — including a
//! group member auto-registered into a group allowlist — could invoke any
//! installed skill and have it execute under the session's approval policy.
//! Under auto-approve that is full tool execution triggered by a non-owner.
//!
//! The gate matches FIRST and refuses after, so an unmatched slash still gets
//! the ordinary "Unknown command" reply and the refusal never reveals which
//! commands or skills exist.
//!
//! These tests drive `gate_user_command` rather than `match_user_command`,
//! which loads from disk. That untestability is why the path shipped without a
//! gate and stayed uncovered: the existing owner-gate tests only ever passed
//! `is_owner = true`.

use crate::brain::commands::UserCommand;
use crate::brain::skills::{Skill, SkillSource};
use crate::channels::commands::{ChannelCommand, gate_user_command, match_user_command_inner};

fn deploy_command() -> UserCommand {
    UserCommand {
        name: "/deploy".to_string(),
        description: "Ship the current branch".to_string(),
        action: "prompt".to_string(),
        prompt: "Deploy the current branch to production.".to_string(),
    }
}

fn status_command() -> UserCommand {
    UserCommand {
        name: "/notice".to_string(),
        description: "Canned notice".to_string(),
        action: "system".to_string(),
        prompt: "Maintenance window is open.".to_string(),
    }
}

fn audit_skill() -> Skill {
    Skill {
        name: "security-audit".to_string(),
        slash_name: "/security-audit".to_string(),
        description: "Audit the repo".to_string(),
        body: "Run a full security audit and report findings.".to_string(),
        globs: Vec::new(),
        review_gate: false,
        source: SkillSource::User,
    }
}

fn assert_owner_refusal(cmd: ChannelCommand, what: &str) {
    match cmd {
        ChannelCommand::UnknownCommand(msg) => assert!(
            msg.to_lowercase().contains("owner"),
            "{what}: denial should read as owner-only, got: {msg}"
        ),
        _ => panic!("{what} must be refused for a non-owner"),
    }
}

/// The reported hole: a commands.toml entry executed by a non-owner.
#[test]
fn a_user_command_is_refused_for_a_non_owner() {
    let matched = match_user_command_inner("/deploy", &[deploy_command()], &[]);
    assert!(
        matches!(matched, ChannelCommand::UserPrompt(_)),
        "fixture must match"
    );
    assert_owner_refusal(gate_user_command(matched, false), "/deploy");
}

/// `action = "system"` is gated too: it still reaches the channel as bot output.
#[test]
fn a_system_action_command_is_refused_for_a_non_owner() {
    let matched = match_user_command_inner("/notice", &[status_command()], &[]);
    assert!(
        matches!(matched, ChannelCommand::UserSystem(_)),
        "fixture must match"
    );
    assert_owner_refusal(gate_user_command(matched, false), "/notice");
}

/// Skills are the higher-value target: the body executes under the session's
/// approval policy.
#[test]
fn a_skill_slug_is_refused_for_a_non_owner() {
    let matched = match_user_command_inner("/security-audit", &[], &[audit_skill()]);
    assert!(
        !matches!(matched, ChannelCommand::UnknownCommand(_)),
        "fixture must match a skill"
    );
    assert_owner_refusal(gate_user_command(matched, false), "/security-audit");
}

/// Arguments must not smuggle a command past the gate.
#[test]
fn arguments_do_not_bypass_the_gate() {
    let matched = match_user_command_inner("/deploy staging --force", &[deploy_command()], &[]);
    assert_owner_refusal(gate_user_command(matched, false), "/deploy with args");
}

/// The owner keeps every one of them, unchanged.
#[test]
fn the_owner_is_unaffected() {
    for (text, cmds, skills) in [
        ("/deploy", vec![deploy_command()], vec![]),
        ("/notice", vec![status_command()], vec![]),
        ("/security-audit", vec![], vec![audit_skill()]),
    ] {
        let matched = match_user_command_inner(text, &cmds, &skills);
        let gated = gate_user_command(matched, true);
        assert!(
            !matches!(gated, ChannelCommand::UnknownCommand(_)),
            "{text} must still work for the owner"
        );
    }
}

/// No existence leak: an unmatched slash gets the ordinary reply, so a
/// non-owner cannot probe which commands or skills are installed by reading
/// which refusal comes back.
#[test]
fn an_unmatched_slash_is_not_turned_into_an_owner_refusal() {
    let matched = match_user_command_inner("/no-such-thing", &[deploy_command()], &[audit_skill()]);
    let gated = gate_user_command(matched, false);
    match gated {
        ChannelCommand::UnknownCommand(msg) => assert!(
            !msg.to_lowercase().contains("owner"),
            "an unmatched slash must not answer with the owner refusal, or the \
             refusal itself enumerates what exists: {msg}"
        ),
        _ => panic!("unmatched slash should stay UnknownCommand"),
    }
}

/// The refusal is byte-identical to the built-ins', so a non-owner cannot tell
/// a gated user command from a gated built-in.
#[test]
fn the_refusal_is_indistinguishable_from_a_built_in_refusal() {
    let matched = match_user_command_inner("/deploy", &[deploy_command()], &[]);
    match gate_user_command(matched, false) {
        ChannelCommand::UnknownCommand(msg) => {
            assert_eq!(msg, "🔒 Owner-only command.");
        }
        _ => panic!("expected an owner refusal"),
    }
}
