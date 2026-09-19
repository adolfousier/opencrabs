//! Surface relevant memory without the model having to ask (#799).
//!
//! MEMORY.md was written constantly and read almost never. #800 made reading
//! cheap, but a cheap read still has to be chosen, and the case that hurts most
//! is the one where the model does not know there is anything to look up. It
//! cannot decide to recall a correction it has forgotten exists.
//!
//! So recall runs on the ENTRY path instead: the user's message is matched
//! against MEMORY.md at turn start and anything relevant rides along with it.
//! This mirrors `hints.rs`, which already proved the shape works, except that
//! fires on the error path after a tool call has already missed.
//!
//! Deliberately conservative. This is paid on every single turn and competes
//! with the actual task for attention, so it stays silent unless the match is
//! good, and it never grows past a couple of short sections.

use crate::brain::brain_sections;
use crate::brain::section_rank::Ranked;

/// At most this many sections ride along with a user message.
pub(crate) const RECALL_MAX_SECTIONS: usize = 2;
/// Total character budget for injected recall.
pub(crate) const RECALL_MAX_CHARS: usize = 1200;
/// Minimum score for a section to ride along: length-normalized BM25 with the
/// IDF scale removed, so the number means the same thing in any workspace.
///
/// Replaces "at least 2 distinct matching terms", which fired on 89.5% of 437
/// real messages because `the`, `and`, `you` and `can` all count as terms and
/// two of them co-occurring is true of almost any section.
///
/// 0.35 comes from the committed fixture in `src/eval/fixtures`, scored both
/// ways: it holds recall at 0.917 while cutting the false-positive rate from
/// 0.417 to 0.250 there, and on a real 156-section MEMORY.md it takes firing on
/// conversational messages from 89.5% down to 17.4% (#996).
pub(crate) const RECALL_MIN_SCORE: f64 = 0.35;

/// Recall relevant to `user_message`, formatted for context, or `None`.
///
/// Pure: takes the file content, so the decision is testable without a home
/// directory or a populated MEMORY.md.
pub fn recall_from(memory: &str, user_message: &str) -> Option<String> {
    if !worth_reading_for(user_message) {
        return None;
    }
    render(Ranked::build(memory).find_relevant(
        user_message,
        RECALL_MAX_SECTIONS,
        RECALL_MAX_CHARS,
        RECALL_MIN_SCORE,
    ))
}

/// Whether `user_message` can possibly produce a recall, decided without
/// touching the disk (#995).
///
/// Both rejections used to happen after the file was already read, so a
/// harness continuation or a message like "ok" paid a full read of a 99 KB
/// file to return `None`. Cheap checks belong before expensive ones.
fn worth_reading_for(user_message: &str) -> bool {
    // A system continuation is the harness talking to itself (restart
    // recovery, nudges). Recall belongs to what the USER asked.
    if user_message.starts_with("[System:") {
        return false;
    }
    !brain_sections::query_terms(user_message).is_empty()
}

/// The parsed MEMORY.md behind the cache, with what it was parsed from.
struct Cached {
    /// Identity of the file this was built from. A changed mtime or length
    /// invalidates it. Length is carried too because mtime resolution is
    /// coarse enough that a same-second rewrite can otherwise go unnoticed.
    stamp: (std::time::SystemTime, u64),
    indexed: Ranked,
}

static CACHE: std::sync::RwLock<Option<Cached>> = std::sync::RwLock::new(None);

/// Touch the epistemic beliefs for recalled sections (#1641).
///
/// When a section is injected into context via recall, its corresponding
/// belief gets a hit. Uses the same key format as `backfill_from_content`:
/// `"MEMORY.md##<heading>"`.
fn touch_recalled(matches: &brain_sections::Matches) {
    for section in &matches.sections {
        let heading = section.heading.trim().trim_start_matches('#').trim();
        if !heading.is_empty() {
            let key = format!("MEMORY.md##{}", heading);
            crate::brain::tools::epistemic::touch_belief(&key);
        }
    }
}

/// Recall relevant to `user_message` from MEMORY.md on disk.
///
/// `None` when the message cannot match, the file is absent or unreadable, or
/// nothing matched well enough.
///
/// The parse is cached and reused until the file changes, so the steady-state
/// cost per turn is a metadata stat plus a set lookup per section, not a read
/// and re-tokenization of the whole file.
pub async fn recall_for(user_message: &str) -> Option<String> {
    if !worth_reading_for(user_message) {
        return None;
    }

    let path = crate::config::opencrabs_home().join("MEMORY.md");
    let meta = tokio::fs::metadata(&path).await.ok()?;
    let stamp = (meta.modified().ok()?, meta.len());

    // Fast path: the file has not changed since it was parsed.
    {
        let cache = CACHE.read().ok()?;
        if let Some(c) = cache.as_ref()
            && c.stamp == stamp
        {
            let matches = c.indexed.find_relevant(
                user_message,
                RECALL_MAX_SECTIONS,
                RECALL_MAX_CHARS,
                RECALL_MIN_SCORE,
            );
            touch_recalled(&matches);
            return render(matches);
        }
    }

    // Changed or first use. Read off the reactor thread: the indexing path in
    // `memory::index` already does this, and a blocking read of a file this
    // size does not belong on an async worker.
    let memory = tokio::fs::read_to_string(&path).await.ok()?;
    let indexed = Ranked::build(&memory);
    let matches = indexed.find_relevant(
        user_message,
        RECALL_MAX_SECTIONS,
        RECALL_MAX_CHARS,
        RECALL_MIN_SCORE,
    );

    if let Ok(mut cache) = CACHE.write() {
        tracing::debug!(
            "MEMORY.md re-parsed for recall: {} sections, {} bytes",
            indexed.len(),
            memory.len()
        );
        *cache = Some(Cached { stamp, indexed });
    }

    touch_recalled(&matches);
    render(matches)
}

/// Format matches for context, or `None` when nothing matched.
///
/// Labelled as recall, not as something the user said. Without the framing the
/// model can mistake injected memory for part of the request.
fn render(matches: brain_sections::Matches) -> Option<String> {
    if matches.sections.is_empty() {
        return None;
    }
    let body = matches
        .sections
        .iter()
        .map(brain_sections::Section::render)
        .collect::<Vec<_>>()
        .join("\n\n");
    Some(format!(
        "─── from your MEMORY.md, possibly relevant ───\n{body}\n\
         [Recalled automatically. Load MEMORY.md with a query for more.]"
    ))
}
