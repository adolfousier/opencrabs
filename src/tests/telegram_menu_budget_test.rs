//! Regression tests for the Telegram command-menu payload budget (#1613).
//!
//! Telegram silently rejects `setMyCommands` bodies over ~7.8KB with a
//! misleading `BOT_COMMANDS_TOO_MUCH` (empirically bisected: 7750B OK,
//! 7876B rejected). The real 64-command catalog serialized to 8505B and
//! every publish failed from 2026-09-08, freezing the server-side menu
//! without `/clear`. `trim_catalog_to_budget` keeps the catalog under a
//! 7000-byte serialized budget by shortening descriptions in tiers and only
//! then dropping tail commands.

use crate::channels::telegram::agent::{collect_command_catalog, trim_catalog_to_budget};
use teloxide::types::BotCommand;

fn synthetic(n: usize, desc_len: usize) -> Vec<BotCommand> {
    (0..n)
        .map(|i| {
            BotCommand::new(
                format!("zprobe{:02}", i),
                format!("d{}", "x".repeat(desc_len)),
            )
        })
        .collect()
}

fn serialized_len(cmds: &[BotCommand]) -> usize {
    serde_json::to_string(cmds).unwrap().len()
}

#[test]
fn budget_shrinks_descriptions_and_keeps_everything_when_possible() {
    // 64 x 256-char descriptions is the #1613 shape: doesn't fit at any
    // single long tier, but fits once descriptions are shortened.
    let cmds = synthetic(64, 256);
    let out = trim_catalog_to_budget(cmds);
    assert_eq!(out.len(), 64, "all commands should survive via tiering");
    assert!(
        serialized_len(&out) <= 7000,
        "catalog must fit the 7000-byte budget, got {}",
        serialized_len(&out)
    );
    assert!(
        out.iter().all(|c| c.description.chars().count() <= 96),
        "descriptions must be tier-capped"
    );
}

#[test]
fn budget_drops_tail_only_after_all_tiers_exhausted() {
    // 150 commands cannot fit even at 32-char descriptions: tail commands
    // must be dropped, but the head (built-ins are pushed first) survives.
    let cmds = synthetic(150, 256);
    let out = trim_catalog_to_budget(cmds);
    assert!(serialized_len(&out) <= 7000);
    assert!(out.len() < 150, "oversized catalog must shed tail commands");
    assert_eq!(out[0].command, "zprobe00", "head of catalog must survive");
}

#[test]
fn budget_never_returns_empty() {
    let cmds = synthetic(1, 256);
    let out = trim_catalog_to_budget(cmds);
    assert_eq!(out.len(), 1);
}

#[test]
fn budget_preserves_catalog_order() {
    let cmds = synthetic(80, 200);
    let out = trim_catalog_to_budget(cmds);
    let names: Vec<&str> = out.iter().map(|c| c.command.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted, "trimming must not reorder the catalog");
}

#[test]
fn budget_shrinks_descriptions_before_dropping() {
    // 64 x 96-char descriptions fit at the 48 tier without dropping anyone;
    // verify the tier pass alone suffices (no command loss).
    let cmds = synthetic(64, 96);
    let out = trim_catalog_to_budget(cmds);
    assert_eq!(out.len(), 64, "tiering alone must absorb this size");
}

/// The #1613 regression: the REAL catalog (built-ins + commands.toml +
/// skills from the live brain) must serialize under budget so every publish
/// succeeds and `/clear` stays on the server-side menu.
#[test]
fn real_catalog_fits_telegram_payload_budget() {
    let cat = collect_command_catalog();
    assert!(!cat.is_empty());
    assert!(
        serialized_len(&cat) <= 7000,
        "live catalog is {} bytes, budget is 7000 — publishes will fail with \
         BOT_COMMANDS_TOO_MUCH again",
        serialized_len(&cat)
    );
    assert!(
        cat.iter().any(|c| c.command == "clear"),
        "/clear must stay in the published menu"
    );
}
