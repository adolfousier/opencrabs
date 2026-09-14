//! Text-structure detection that gates the rich paths.
//!
//! Pure predicates over the reply text: does it contain a table, a task
//! list, an ATX heading, any block structure worth native rich rendering?
//! The gates live together here so the decision surface is one file: the
//! verdict fns ([`should_send_native_rich`], [`should_send_native_rich_for`])
//! log their inputs, and the raw structure probes they compose
//! ([`contains_table`], [`contains_task_list`], [`has_rich_structure`],
//! [`is_atx_heading`]) stay quiet and cheap.
//!
//! Split out of `mod.rs` when it went declarations-only (#1293 era):
//! functions never live in `mod.rs` (see CONTRIBUTING.md).

use super::{list, table};

/// Whether `text` is better served by the AST renderer than the legacy
/// line-based converter: it contains a GitHub-flavored table or a task-list
/// checkbox, both of which the legacy path renders poorly (raw `| pipes |` and
/// literal `- [ ]` respectively).
pub(crate) fn prefers_rich_render(text: &str) -> bool {
    contains_table(text) || contains_task_list(text) || contains_details(text)
}

/// Whether `text` contains a GitHub-flavored pipe table.
///
/// #132: the scan runs on the NORMALIZED text — collapsed one-line tables are
/// reflowed first, exactly what the rich renderer receives (rich/api.rs calls
/// the same `normalize_tables` entry). Before this, the gate read raw text
/// while the renderer read normalized text: a collapsed table was invisible
/// here (still rich, via other structure) and then rendered as raw pipes —
/// the MIIDAS cron-card defect. Gate and renderer can no longer disagree.
pub(crate) fn contains_table(text: &str) -> bool {
    let normalized = super::normalize_tables(text);
    let lines: Vec<String> = normalized.lines().map(str::to_string).collect();
    (0..lines.len()).any(|i| table::try_parse(&lines, i).is_some())
}

/// Whether a structured reply should be delivered as a native rich message:
/// the `channels.telegram.rich_messages` config flag is on AND the text has
/// block structure ([`has_rich_structure`]). On by default (#425); older
/// clients and Telegram Web show a "not supported" placeholder, so outdated
/// deployments opt out (onboard dialog or `richtext off`) and get the
/// universal HTML rendering. Read via the zero-disk config mirror.
pub(crate) fn should_send_native_rich(text: &str) -> bool {
    should_send_native_rich_for(text, false)
}

/// #45 variant: `has_buttons` forces the rich plane for plain prose that ends
/// on a `suggest_options` surface. The tap rewrite preserves the host plane,
/// so a classic prose host would keep the pick record in plain HTML even
/// though the turn carries interactive buttons — button-bearing answers ride
/// rich instead. The `rich_messages` flag still gates everything (richtext
/// off = never rich, buttons or not).
pub(crate) fn should_send_native_rich_for(text: &str, has_buttons: bool) -> bool {
    let flag = crate::config::Config::current()
        .channels
        .telegram
        .rich_messages;
    let structured = has_rich_structure(text);
    let verdict = flag && (structured || has_buttons);
    // Same visibility rationale as the base verdict (#860): both inputs are
    // recorded so a false verdict says which half caused it — plus the
    // buttons_forced input, so a prose-with-buttons send is distinguishable
    // from a structured send in the log.
    tracing::info!(
        "Telegram rich verdict: {} (rich_messages={}, structured={}, buttons_forced={}, table={}, len={})",
        verdict,
        flag,
        structured,
        has_buttons,
        contains_table(text),
        text.len()
    );
    verdict
}

/// Whether `text` contains block-level markdown structure that native rich
/// rendering handles meaningfully better than plain/HTML: a table, ATX
/// heading, list item, fenced code block, block math, or a `<details>`
/// collapse block — matched by `<details>` line prefix so the inline
/// `<details><summary>` openers count too (the #15 receipt cards emitted that shape before the parser-safe block form).
/// Plain prose (even
/// with inline emphasis) returns false, so it stays on the existing path and
/// is never reinterpreted by Telegram's markdown parser. Gates the native
/// `sendRichMessage` path (together with the config flag).
///
/// A message is NEVER disqualified from rich (the #476 fence-disqualify was
/// reverted: it dragged tables in mixed table+fence messages onto the HTML
/// path, where tables unwrap to raw pipes). Fence mangling under the rich
/// markdown parser is a separate cosmetic issue whose real fix is the
/// native-block serializer (#420 path B), not exclusion.
pub(crate) fn has_rich_structure(text: &str) -> bool {
    contains_table(text)
        || text.lines().any(|line| {
            let t = line.trim_start();
            is_atx_heading(t)
                || list::is_item(t)
                || t.starts_with("```")
                || t.starts_with("~~~")
                || t == "$$"
                || is_details_open(t)
        })
}

/// A `<details>` collapse opener, bare or with attributes (`<details open>`).
/// Shared by the rich gate and the classic HTML ladder so the two can never
/// disagree about who renders a collapse block: [`has_rich_structure`] sends
/// it to `sendRichMessage`, and when that send fails [`prefers_rich_render`]
/// must route the fallback through the rich AST too. The line-based ladder
/// escapes unknown tags, so a collapse that reached it surfaced its literal
/// `<details>` / `<summary>` markup as visible text.
pub(crate) fn is_details_open(t: &str) -> bool {
    t.starts_with("<details>") || t.starts_with("<details ")
}

/// Whether `text` opens a `<details>` collapse block on any line.
pub(crate) fn contains_details(text: &str) -> bool {
    text.lines().any(|line| is_details_open(line.trim_start()))
}

/// A `# `..`###### ` ATX heading line (1-6 hashes followed by a space).
/// Shared by the rich gate and the classic HTML ladder so the two parsers
/// can never disagree on `#N`-style lines (#1257).
pub(crate) fn is_atx_heading(t: &str) -> bool {
    let hashes = t.chars().take_while(|&c| c == '#').count();
    (1..=6).contains(&hashes) && t[hashes..].starts_with(' ')
}

/// Whether `text` contains a `- [ ]` / `- [x]` task-list item.
pub(crate) fn contains_task_list(text: &str) -> bool {
    text.lines().any(|line| {
        let t = line.trim_start();
        let after = t
            .strip_prefix("- ")
            .or_else(|| t.strip_prefix("* "))
            .or_else(|| t.strip_prefix("+ "));
        matches!(after, Some(rest)
            if rest.starts_with("[ ]") || rest.starts_with("[x]") || rest.starts_with("[X]"))
    })
}
