//! Tool-approval buttons must be owner-gated on every channel (OC-01).
//!
//! The approve / always / yolo / deny keyboards sit in a chat where any member
//! can press them. A non-owner tap otherwise runs the pending tool, and a YOLO
//! tap calls `persist_auto_always_policy()`, which flips the whole instance to
//! unattended auto-always across every surface and survives restart. The slash
//! commands and the plan/cd keyboards already re-check the tapper; the
//! tool-approval buttons did not.
//!
//! These callbacks live inside serenity / teloxide / slack-morphism / whatsapp
//! event handlers that a unit test cannot drive without a live socket, so this
//! is a source guard: in each channel handler, every `persist_auto_always_policy`
//! call must be preceded, close by, by an owner check. If someone adds a new
//! persist site or removes the gate, this fails.

use std::path::Path;

/// The channel handlers that own an approval keyboard, and the token each uses
/// for its owner check.
const APPROVAL_HANDLERS: &[&str] = &[
    "src/channels/discord/agent.rs",
    "src/channels/telegram/agent.rs",
    "src/channels/slack/handler.rs",
    "src/channels/whatsapp/handler.rs",
];

/// How many lines back an owner check must appear before a persist call. The
/// gates we ship sit within ~25 lines; 60 leaves room without letting an
/// unrelated check in a different function count.
const MAX_LOOKBACK: usize = 60;

fn owner_check(line: &str) -> bool {
    let code = line.split("//").next().unwrap_or("");
    code.contains("is_owner")
}

#[test]
fn every_yolo_persist_is_owner_gated() {
    let mut offenders = Vec::new();

    for rel in APPROVAL_HANDLERS {
        let text = std::fs::read_to_string(Path::new(rel))
            .unwrap_or_else(|e| panic!("{rel} must be readable ({e}); did the handler move?"));
        let lines: Vec<&str> = text.lines().collect();

        for (i, line) in lines.iter().enumerate() {
            // The call itself, not this guard's prose or a doc comment.
            let code = line.split("//").next().unwrap_or("");
            if !code.contains("persist_auto_always_policy") {
                continue;
            }
            let lo = i.saturating_sub(MAX_LOOKBACK);
            let gated = lines[lo..=i].iter().any(|l| owner_check(l));
            if !gated {
                offenders.push(format!("{rel}:{}", i + 1));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these persist_auto_always_policy calls have no owner check within {MAX_LOOKBACK} lines \
         above them, so a non-owner button tap could flip the instance to auto-always (OC-01):\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn the_guard_actually_sees_the_persist_calls() {
    // Guards that match nothing pass vacuously. Prove the token is present in
    // the tree so a rename of the function cannot silently disarm this test.
    let mut found = 0usize;
    for rel in APPROVAL_HANDLERS {
        let text = std::fs::read_to_string(Path::new(rel)).unwrap();
        found += text.matches("persist_auto_always_policy").count();
    }
    assert!(
        found >= APPROVAL_HANDLERS.len(),
        "expected at least one persist call per channel handler, found {found}; \
         the guard above may be measuring nothing"
    );
}
