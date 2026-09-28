//! Channel capabilities preamble (#1773, port of fork #295 with the fork
//! #513 fence fix folded in).
//!
//! What is pinned here and why:
//!
//! 1. **Placement + idempotence.** The injected block lands before
//!    `--- Runtime Info ---` (or appends when absent), and re-injection is a
//!    no-op. A duplicated block would re-bill tokens on every turn.
//! 2. **The fence example must be one the renderer accepts (#513).** The
//!    mermaid resolvers trim the fence info string and accept EXACTLY
//!    `mermaid` (src/utils/mermaid.rs:95,144; telegram rich mermaid
//!    :1123). An example like "```mermaid vertical" ships the diagram as an
//!    ordinary code fence: no render, no image, and no error.
//! 3. **The loader stays channel-agnostic.** Channel awareness is a
//!    per-session runtime decision; one process serves Telegram, Discord,
//!    Slack and cron alike, so the startup brain must carry no block.
//! 4. **The #1773 directive rides both blocks.** A channel user has no
//!    filesystem: research/report files must be attached to the channel,
//!    not just named.
//! 5. **Survival across compaction.** Upstream welds nothing onto the
//!    summary (#1649/#1676) because `system_brain` survives compaction
//!    untouched: that property is asserted here, so a future compaction
//!    rework cannot silently drop the block.

use crate::brain::agent::context::{AgentContext, CompactionScope};
use crate::brain::agent::service::AgentService;
use crate::brain::prompt_builder::{
    BrainLoader, TELEGRAM_CHANNEL_CAPABILITIES_MARKER, channel_file_delivery_capabilities,
    has_channel_file_delivery, has_telegram_channel_capabilities, inject_channel_capabilities,
    inject_telegram_channel_capabilities, telegram_channel_capabilities,
};
use tempfile::TempDir;

#[test]
fn test_inject_telegram_channel_capabilities_explicit() {
    let brain_with_runtime = "You are OpenCrabs.\n\n--- Runtime Info ---\nModel: test\n";
    let injected = inject_telegram_channel_capabilities(brain_with_runtime);
    assert!(has_telegram_channel_capabilities(&injected));
    // #513: the preamble must demonstrate the fence tag the renderer accepts.
    assert!(
        injected.contains("tag the fence exactly ```mermaid"),
        "the preamble must show the exact fence tag the renderer accepts"
    );
    assert!(
        !injected.contains("```mermaid "),
        "no mermaid example may carry a suffixed info string: the renderer \
         trims it and accepts only exactly mermaid"
    );
    assert!(injected.contains("- Markdown tables: GFM tables rendered natively"));
    assert!(injected.contains("- HTML glyphs / formatting"));
    assert!(injected.contains("- Image includes: Markdown syntax"));
    // #1773: the file-delivery directive rides the block.
    assert!(injected.contains("- Report/research files:"));

    // Ensure it was placed before Runtime Info
    let cap_pos = injected.find(TELEGRAM_CHANNEL_CAPABILITIES_MARKER).unwrap();
    let runtime_pos = injected.find("--- Runtime Info ---").unwrap();
    assert!(cap_pos < runtime_pos);

    // Idempotent injection
    let injected_again = inject_telegram_channel_capabilities(&injected);
    assert_eq!(injected, injected_again);
}

#[test]
fn test_inject_telegram_channel_capabilities_without_runtime_info() {
    let plain_brain = "You are OpenCrabs.\n";
    let injected = inject_telegram_channel_capabilities(plain_brain);
    assert!(has_telegram_channel_capabilities(&injected));
    assert!(injected.starts_with("You are OpenCrabs."));
}

#[test]
fn test_brain_loader_does_not_inject_channel_capabilities() {
    // Channel awareness is a per-session runtime decision (#295/#1773), not
    // a property of the startup brain: one process serves Telegram, Discord,
    // Slack and cron sessions alike, so the loader must stay channel-agnostic.
    let temp_dir = TempDir::new().unwrap();
    let loader = BrainLoader::new(temp_dir.path().to_path_buf());

    assert!(!has_telegram_channel_capabilities(
        &loader.build_core_brain(None)
    ));
    assert!(!has_telegram_channel_capabilities(
        &loader.build_system_brain(None)
    ));
    assert!(!has_channel_file_delivery(&loader.build_core_brain(None)));
}

#[test]
fn test_channel_capabilities_dispatcher() {
    let brain = "You are OpenCrabs.\n\n--- Runtime Info ---\n";

    // Telegram-bound: renderer capabilities block.
    let telegram = inject_channel_capabilities(brain, true, true);
    assert!(has_telegram_channel_capabilities(&telegram));
    assert!(!has_channel_file_delivery(&telegram));

    // Channel-bound but not Telegram: generic file-delivery block only.
    let generic = inject_channel_capabilities(brain, false, true);
    assert!(has_channel_file_delivery(&generic));
    assert!(!has_telegram_channel_capabilities(&generic));

    // Unbound (TUI, cron): neither block.
    let unbound = inject_channel_capabilities(brain, false, false);
    assert_eq!(unbound, brain);
}

#[test]
fn test_file_delivery_directive_in_both_blocks() {
    let telegram = telegram_channel_capabilities();
    let generic = channel_file_delivery_capabilities();
    for block in [&telegram, &generic] {
        assert!(
            block.contains("attach the file to this channel"),
            "every channel capabilities block must carry the #1773 delivery directive"
        );
        assert!(
            block.contains("files before the final text"),
            "the directive must reference the ORDERING preamble's timing rule"
        );
    }
    assert!(
        !generic.contains("Mermaid diagrams"),
        "the generic block must not claim Telegram renderer facts"
    );
}

#[test]
fn test_compaction_preserves_capabilities_in_system_brain() {
    // Upstream welds nothing onto the summary (#1649/#1676): system_brain
    // survives compaction untouched, which is WHY the injection needs no
    // post-compaction re-wire (the fork needed one; upstream does not).
    // Pin the property so a compaction rework cannot silently drop it.
    let mut context = AgentContext::new(uuid::Uuid::new_v4(), 100_000);
    context.system_brain = Some(format!(
        "You are OpenCrabs.\n\n{}\n\n--- Runtime Info ---\n",
        telegram_channel_capabilities()
    ));

    AgentService::apply_scoped_compaction_summary(
        &mut context,
        CompactionScope::FullWindow,
        "Summary of previous tasks.",
    );

    let brain = context.system_brain.as_deref().expect("system brain kept");
    assert!(brain.contains(TELEGRAM_CHANNEL_CAPABILITIES_MARKER));
    assert!(brain.contains("- Report/research files:"));
}
