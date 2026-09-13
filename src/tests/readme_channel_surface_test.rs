//! The README's channel claims, checked against the code they describe.
//!
//! Two claims here have now drifted twice: the `whatsapp_send` row said 26
//! actions when 27 shipped, and the `telegram_send` row beside it said 19 when
//! 20 shipped. Both are one number in a table that nobody re-derives while
//! adding an action, so both will drift again. A reader cannot tell a stale
//! count from a true one, which makes the table worse than no table.
//!
//! These tests re-derive the numbers from the tool schemas at test time, so a
//! new action fails the suite instead of quietly aging the docs.

use std::path::Path;

fn read(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {rel}: {e}"))
}

/// Count the entries of the `"enum": [...]` action list in a tool's schema.
///
/// Reads the FIRST such array, which is the `action` parameter in every
/// channel-send tool; later enums (if any) belong to other parameters.
fn declared_actions(tool_src: &str) -> Vec<String> {
    let start = tool_src
        .find("\"enum\": [")
        .expect("tool schema has an action enum");
    let rest = &tool_src[start..];
    let end = rest.find(']').expect("action enum is closed");
    rest[..end]
        .split('"')
        .map(str::trim)
        .filter(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
        .filter(|t| *t != "enum")
        .map(str::to_string)
        .collect()
}

/// The number a README row claims, e.g. `| \`whatsapp_send\` | 27 actions: ...`.
fn claimed_actions(readme: &str, tool: &str) -> usize {
    let row = readme
        .lines()
        .find(|l| l.contains(&format!("`{tool}`")) && l.contains(" actions"))
        .unwrap_or_else(|| panic!("README has no action-count row for {tool}"));
    let after = row.split_once(" actions").expect("row names a count").0;
    after
        .rsplit(|c: char| !c.is_ascii_digit())
        .find(|t| !t.is_empty())
        .expect("count is a number")
        .parse()
        .expect("count parses")
}

#[test]
fn readme_action_counts_match_the_tool_schemas() {
    let readme = read("README.md");
    for (tool, src) in [
        ("whatsapp_send", "src/brain/tools/whatsapp_send.rs"),
        ("telegram_send", "src/brain/tools/telegram_send.rs"),
    ] {
        let actual = declared_actions(&read(src)).len();
        let claimed = claimed_actions(&readme, tool);
        assert_eq!(
            claimed, actual,
            "README says {tool} has {claimed} actions; {src} declares {actual}. \
             Update the README row in the same commit as the action."
        );
    }
}

#[test]
fn every_whatsapp_action_is_reachable() {
    // A schema entry with no match arm is an action the model will call and
    // the tool will reject as unknown, which reads to the user as the agent
    // hallucinating a capability the docs promised.
    let src = read("src/brain/tools/whatsapp_send.rs");
    for action in declared_actions(&src) {
        assert!(
            src.contains(&format!("\"{action}\" =>")) || src.contains(&format!("\"{action}\" |")),
            "action '{action}' is in the schema with no match arm"
        );
    }
}

#[test]
fn every_whatsapp_config_key_is_documented() {
    // `interactive_buttons` shipped undocumented, and it is the key that gates
    // whether a safety-critical approval prompt takes an unproven render path.
    // A config key nobody can find is a decision nobody can review.
    let types = read("src/config/types.rs");
    let readme = read("README.md");
    // From after the opening brace, not from the declaration: `pub struct
    // WhatsAppConfig {` also starts with `pub `, and reading it as a field
    // asks the README to document a key named "struct WhatsAppConfig {".
    let decl = types
        .find("pub struct WhatsAppConfig")
        .expect("WhatsAppConfig exists");
    let open = decl + types[decl..].find('{').expect("struct has a body") + 1;
    let body = &types[open..open + types[open..].find("\n}").expect("struct closes")];

    for line in body.lines() {
        let Some(field) = line.trim().strip_prefix("pub ") else {
            continue;
        };
        let Some(name) = field.split(':').next() else {
            continue;
        };
        let name = name.trim();
        // Shared by all eight channel config structs and documented for none
        // of them, so it belongs to a cross-channel docs pass rather than to
        // this one. Named here rather than skipped silently.
        if name == "session_idle_hours" {
            continue;
        }
        assert!(
            readme.contains(name),
            "[channels.whatsapp] key '{name}' appears nowhere in README.md"
        );
    }
}
