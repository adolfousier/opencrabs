use std::path::{Path, PathBuf};

/// Extract `<<IMG:path>>` markers from text.
///
/// Returns `(cleaned_text, vec_of_paths)` — the text has all markers removed
/// and trimmed, the vec contains the file paths in order of appearance.
pub fn extract_img_markers(text: &str) -> (String, Vec<String>) {
    extract_markers_with_prefix(text, "<<IMG:")
}

/// Extract `<<VID:path>>` markers from text — mirror of `extract_img_markers`
/// for video attachments. Used by channel handlers to strip the marker from
/// bot replies before display (the agent shouldn't normally echo it back, but
/// strip defensively so a leaking marker never lands in front of the user).
pub fn extract_vid_markers(text: &str) -> (String, Vec<String>) {
    extract_markers_with_prefix(text, "<<VID:")
}

/// Extract `<<react:emoji>>` directive from text.
///
/// Returns `(cleaned_text, Option<emoji>)` — valid directives are removed
/// (text trimmed) and the first extracted emoji is returned. Multiple valid
/// directives are all stripped but only the first emoji is returned.
///
/// The LLM outputs `<<react:👍>>` to signal a reaction-only response
/// (or a reaction alongside text). Channel handlers use the returned
/// emoji to call `set_message_reaction` on the user's message.
///
/// Both ends of the marker are matched tolerantly. The opening prefix: some
/// models escape the angle brackets and emit `<\react:` or `<\\react:` instead
/// of `<<react:`, and some drop the `react:` tag entirely and just double-
/// bracket the emoji, `<<✅>>` (see `match_react_open`). The closing terminator:
/// `>>`, an XML-style `</react>`, or a bare `>` all close the directive (see
/// `find_react_close`) — models trained on Cursor/Cline-style harnesses close
/// directives with `</tag>`, and that leaked mangled markers as raw text when
/// only `>>` was accepted. All of these normalize to the same extraction, so
/// the reaction still fires and the mangled marker never leaks into the chat as
/// raw text.
///
/// Unlike the `<<IMG:path>>` extractor this is deliberately strict, because
/// the marker can legitimately appear in PROSE when the agent talks about
/// the feature itself (docs, code review, this codebase). Two guards:
/// * the payload must look like an actual emoji (non-empty, ≤ 8 chars, no
///   ASCII) — `<<react:emoji>>` or `<<react:hello>>` written in prose stays
///   in the text and produces no reaction (a word payload once fired a bogus
///   REACTION_INVALID Telegram call and mutated the final text, breaking
///   exact-match dedup against the already-sent intermediate: both copies
///   of the message landed in the chat);
/// * occurrences inside backtick code spans are never treated as directives.
pub fn extract_react_marker(text: &str) -> (String, Option<String>) {
    extract_react_marker_inner(text, true)
}

/// Like [`extract_react_marker`] but ignores backtick code spans — a marker
/// inside `` `…` `` still fires and is stripped. Use ONLY where the marker is
/// known to be a genuine directive, not prose that might discuss the feature:
/// a reaction-notification turn's response, whose expected output IS a bare
/// `<<react:emoji>>`. Small models there wrap the marker in a code span and
/// narrate their reasoning, so the strict extractor misses it — leaving the
/// marker as visible `<code>` text and firing no reaction.
pub fn extract_react_marker_lenient(text: &str) -> (String, Option<String>) {
    extract_react_marker_inner(text, false)
}

/// Reaction-path-only companion of [`extract_react_marker_lenient`] (#1670).
///
/// Removes every remaining marker-shaped occurrence whose payload fails
/// [`is_reaction_emoji`] — the empty `<<react:>>` (a model that forgot its
/// emoji), word payloads, keyword-less `<<noise>>` brackets. Such a marker
/// survives extraction as visible text and used to be delivered as a bubble;
/// in a forum group that bubble landed in General, in front of a dormant
/// topic (#1670). A reaction turn's output is a directive or the model's
/// narration — never prose discussing the feature — so unlike the strict
/// extractor this strips regardless of code spans and has no prose guard.
/// Valid markers are LEFT (the extractor already consumed them; leaving them
/// keeps this function idempotent-safe if ever called first). Unterminated
/// openings (`<<react:` with no terminator) are LEFT: there is no close to
/// anchor a strip to, and eating the rest of the text would be a worse leak.
pub fn strip_invalid_react_markers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let ch = text[i..].chars().next().expect("i lies on a char boundary");
        if ch == '<'
            && let Some(open_len) = match_react_open(&text[i..])
            && let Some((rel_end, term_len)) = find_react_close(&text[i + open_len..])
        {
            let payload = text[i + open_len..i + open_len + rel_end].trim();
            if !is_reaction_emoji(payload) {
                i += open_len + rel_end + term_len; // debris: drop the marker
                // A code-wrapped marker (`<<react:>>` in backticks) would
                // strand its delimiters as "``" — a still-visible empty
                // bubble. Pop one adjacent backtick from each side so the
                // whole span disappears and the turn degrades to silence.
                if out.ends_with('`') && text[i..].starts_with('`') {
                    out.pop();
                    i += 1;
                }
                // Collapse the spaces that framed the marker: debris in
                // mid-sentence must leave one gap, not two. (The extractor
                // never cared because its narration is dropped wholesale;
                // a stripped marker keeps the narration alive.)
                if out.ends_with(' ') && text[i..].starts_with(' ') {
                    i += 1;
                }
                continue;
            }
        }
        out.push(ch);
        i += ch.len_utf8();
    }
    out.trim().to_string()
}

fn extract_react_marker_inner(
    text_arg: &str,
    respect_code_spans: bool,
) -> (String, Option<String>) {
    // Pass 1: the plain scanner. Fires on canonical turns and preserves all
    // code-span semantics; the ONLY path taken when the text has no backticks.
    let plain = scan_react_text(text_arg, respect_code_spans);
    if plain.1.is_some() || !text_arg.contains('`') {
        return plain;
    }
    // Pass 2 (#1182): strict found nothing and backticks are present — try
    // orphan-fence recovery. A full junk-prefixed directive means the text is
    // a mangled REACTION TURN; recovery captures its emoji and returns the
    // remaining body. Prose about the feature disqualifies itself (real words
    // sit before the marker), so docs examples stay text. Recovery runs AFTER
    // strict, never instead of it: an empty prefix is legal fence-junk, so a
    // recovery-first order would eat every well-formed marker that merely
    // carries a trailing code fence.
    match recover_orphan_fenced_directive(text_arg) {
        Some((cleaned, emoji)) => (cleaned.trim().to_string(), Some(emoji)),
        None => plain,
    }
}

/// Single left-to-right scan extracting the first reaction marker, honouring
/// the code-span guard when `respect_code_spans` is set. Shared by both
/// passes of [`extract_react_marker_inner`].
fn scan_react_text(text: &str, respect_code_spans: bool) -> (String, Option<String>) {
    let mut out = String::with_capacity(text.len());
    let mut emoji: Option<String> = None;
    let mut in_code = false;
    let mut i = 0;

    while i < text.len() {
        let ch = text[i..].chars().next().expect("i lies on a char boundary");
        if ch == '`' {
            in_code = !in_code;
            out.push(ch);
            i += 1;
            continue;
        }
        if (!respect_code_spans || !in_code)
            && let Some(open_len) = match_react_open(&text[i..])
            && let Some((rel_end, term_len)) = find_react_close(&text[i + open_len..])
        {
            let payload = text[i + open_len..i + open_len + rel_end].trim();
            if is_reaction_emoji(payload) {
                if emoji.is_none() {
                    emoji = Some(payload.to_string());
                }
                i += open_len + rel_end + term_len; // past the terminator
                continue;
            }
        }
        out.push(ch);
        i += ch.len_utf8();
    }

    (out.trim().to_string(), emoji)
}

/// Recovery pass for reaction directives mangled by an orphan code fence (#1182).
///
/// Shape observed in production: the message OPENS with junk made only of
/// fence/lang tokens (`<`, backticks, `\`, `~`, whitespace, an ascii-alnum
/// language tag like `html`) and a valid directive sits inside that junk
/// region. That prefix shape cannot be legitimate prose about the feature
/// (real discussion puts the marker after words like "use" or ":"), so a full
/// match (open + terminator + emoji payload) found there means the text is a
/// mangled REACTION TURN: slice past the junk and drop the paired trailing
/// lone-fence line. Anything else returns borrowed and unchanged.
/// True when everything before the directive is orphan-fence debris: an
/// optional stray `<` or `\`, an opening ``` fence with an optional short
/// language tag, then only whitespace/backticks/angle-brackets. Any real
/// word (letters outside the lang-tag slot) disqualifies, so prose like
/// ``use `<<react:👍>>` to react`` keeps its code-span semantics (#1182).
fn is_fence_junk_prefix(prefix: &str) -> bool {
    let mut rest = prefix.trim_start_matches([' ', '\t', '\r', '\n']);
    if let Some(r) = rest.strip_prefix('<').or_else(|| rest.strip_prefix('\\')) {
        rest = r.trim_start_matches([' ', '\t', '\r', '\n']);
    }
    if let Some(r) = rest.strip_prefix("```") {
        rest = r;
        let tag_len = rest.chars().take_while(|c| c.is_alphanumeric()).count();
        if tag_len > 12 {
            return false;
        }
        rest = &rest[tag_len..];
    }
    rest.bytes()
        .all(|b| matches!(b, b'`' | b'<' | b'\\' | b' ' | b'\t' | b'\r' | b'\n'))
}

/// Recovery pass for reaction directives mangled by an orphan code fence (#1182).
///
/// Shape observed in production: the message OPENS with junk made only of
/// fence/lang tokens (`<`, backticks, `\`, whitespace, an ascii-alnum
/// language tag like `html`) and a valid directive sits inside that junk
/// region. That prefix shape cannot be legitimate prose about the feature
/// (real discussion puts the marker after words like "use" or ":"), so a full
/// match (open + terminator + emoji payload) found there means the text is a
/// mangled REACTION TURN. Returns the body after the directive with the paired
/// trailing lone-fence line dropped, plus the captured emoji; `None` when no
/// junk-prefixed directive exists.
fn recover_orphan_fenced_directive(text: &str) -> Option<(String, String)> {
    if !text.contains('`') {
        return None;
    }
    let scan_end = text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .find(|&i| i >= 240)
        .unwrap_or(text.len());
    let mut found: Option<(usize, String)> = None;
    for (i, ch) in text[..scan_end].char_indices() {
        if ch != '<' {
            continue;
        }
        if !is_fence_junk_prefix(&text[..i]) {
            continue;
        }
        if let Some(open_len) = match_react_open(&text[i..])
            && let Some((rel_end, term_len)) = find_react_close(&text[i + open_len..])
            && is_reaction_emoji(text[i + open_len..i + open_len + rel_end].trim())
        {
            let emoji = text[i + open_len..i + open_len + rel_end]
                .trim()
                .to_string();
            found = Some((i + open_len + rel_end + term_len, emoji));
            break;
        }
    }
    let (after, emoji) = found?;
    let mut owned = text[after..].to_string();
    if let Some(pos) = owned.rfind('\n')
        && owned[pos + 1..].trim_start().starts_with("```")
    {
        owned.truncate(pos);
    }
    Some((owned, emoji))
}

/// Match a reaction-marker opening at the start of `s`, tolerating prefixes
/// mangled by models that escape the angle brackets. Accepts a leading `<`
/// followed by any run of `<` or `\` characters, then `react:`, so the
/// canonical `<<react:` as well as `<\react:`, `<\\react:`, and `<react:` all
/// match. Returns the byte length of the matched opening (through `react:`),
/// or `None` when `s` does not begin with a marker opening.
///
/// Also matches the keyword-LESS form `<<EMOJI>>`: some models drop the
/// `react:` tag entirely and just bracket the emoji. That form is accepted only
/// when the leading run has at least two `<` characters — a single-bracket
/// `<x>` is one char from HTML/emoticon noise (and one char from a bare-`>`
/// terminator), so it must stay prose. The payload is NOT validated here; the
/// caller's `is_reaction_emoji` guard still rejects word payloads, so
/// `<<hello>>` stays text and only a real `<<✅>>` fires.
///
/// All matched bytes are ASCII (`<`, `\`, `react:`), so the returned length
/// always lands on a char boundary.
fn match_react_open(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'<') {
        return None;
    }
    let mut j = 1;
    let mut angle_brackets = 1usize; // bytes[0] is '<'
    while let Some(c) = bytes.get(j) {
        match c {
            b'<' => {
                angle_brackets += 1;
                j += 1;
            }
            b'\\' => j += 1,
            _ => break,
        }
    }
    const TAG: &str = "react:";
    if s[j..].starts_with(TAG) {
        // Canonical keyword form: `<<react:` (any bracket count).
        Some(j + TAG.len())
    } else if angle_brackets >= 2 {
        // Keyword-less `<<EMOJI>>` — payload validated by the caller.
        Some(j)
    } else {
        None
    }
}

/// Find the earliest reaction-marker terminator in `s`, tolerating the strict
/// `>>` close as well as the `</react>` (XML-style close tag) and bare `>`
/// variants that models emit when they mangle the directive. Returns
/// `(offset, term_len)` — the byte offset where the terminator starts and its
/// byte length — or `None` when none is present.
///
/// When more than one candidate starts at the SAME offset the longest wins, so
/// a canonical `>>` is never mis-read as a bare `>` (which would strand the
/// trailing bracket in the output). All terminators are ASCII, so both the
/// offset and `offset + term_len` land on char boundaries.
fn find_react_close(s: &str) -> Option<(usize, usize)> {
    const TERMS: [&str; 3] = [">>", "</react>", ">"];
    let mut best: Option<(usize, usize)> = None;
    for term in TERMS {
        if let Some(pos) = s.find(term) {
            let better = match best {
                Some((bpos, blen)) => pos < bpos || (pos == bpos && term.len() > blen),
                None => true,
            };
            if better {
                best = Some((pos, term.len()));
            }
        }
    }
    best
}

/// A plausible reaction emoji: non-empty, short (compound emoji with skin
/// tones / VS-16 / ZWJ stay under 8 chars), and containing no ASCII — which
/// rejects words and placeholders like "emoji" or "hello" that appear when
/// the marker is mentioned in prose rather than used as a directive.
fn is_reaction_emoji(payload: &str) -> bool {
    !payload.is_empty() && payload.chars().count() <= 8 && payload.chars().all(|c| !c.is_ascii())
}

/// Generic `<<PREFIX:path>>` marker extractor. Walks the text, removes every
/// `<<PREFIX:...>>` occurrence, and collects the inner paths in order. UTF-8
/// safe (works on byte indices that lie on char boundaries — `find`/`replace_range`
/// handle that correctly for the ASCII delimiters used here).
fn extract_markers_with_prefix(text: &str, prefix: &str) -> (String, Vec<String>) {
    let mut out = text.to_string();
    let mut paths = Vec::new();
    let prefix_len = prefix.len();

    while let Some(start) = out.find(prefix) {
        let Some(rel_end) = out[start..].find(">>") else {
            break;
        };
        let end = start + rel_end + 2; // past ">>"
        let path = out[start + prefix_len..start + rel_end].trim().to_string();
        if !path.is_empty() {
            paths.push(path);
        }
        out.replace_range(start..end, "");
    }

    (out.trim().to_string(), paths)
}

// ── Local-file link delivery (#1916) ───────────────────────────────────────
//
// A reply can name a file on this host with an ordinary markdown link,
// `[label](/srv/reports/q3.pdf)`. Until now that link reached the chat as
// dead text: the label rendered, the target went nowhere, and the file never
// left the machine. This section scans a reply for such links, validates the
// targets, and hands the resolved files to the channel delivery layer, which
// ships each one as a document bubble.
//
// One policy rules the whole family: a candidate is removed from the reply
// text ONLY when it resolved, validated, and will actually be delivered. A
// REJECTED candidate (missing file, empty file, unreadable) stays in the text
// byte-identical AND is reported as a failure, because a link carries its own
// label and a silent strip would delete the reader's only clue about what was
// referenced. A non-file target (remote URL, `mailto:`, in-page anchor,
// Telegram media ref) is left as written: Telegram resolves those itself.

/// Why a candidate reference was rejected. One taxonomy shared by every media
/// family, so a notice can name the reason without the channel layer knowing
/// how validation works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalImageFailureReason {
    /// The path does not exist.
    NotFound,
    /// The path exists but is not a regular file (a directory, a socket).
    NotAFile,
    /// The file exists but holds 0 bytes.
    Empty,
    /// The file exists but could not be opened or read.
    Unreadable,
    /// More bytes than the family's size ceiling.
    TooLarge,
    /// The attachment was extracted and validated, but the channel refused to
    /// send it (API error, media-type rejection, platform size ceiling).
    /// Distinct from every other reason: the reference is not the problem.
    DeliveryFailed,
}

impl LocalImageFailureReason {
    /// Short human- and model-facing phrase used in notices.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "file not found",
            Self::NotAFile => "not a regular file",
            Self::Empty => "file is empty (0 bytes)",
            Self::Unreadable => "file could not be read",
            Self::TooLarge => "larger than the size limit",
            Self::DeliveryFailed => "the channel could not deliver it",
        }
    }
}

/// One reference that was removed from the reply but could not be delivered,
/// with the reason the model needs in order to fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalImageFailure {
    /// The reference exactly as it appeared in the reply text.
    pub raw: String,
    /// The path the reference resolved to, when resolution succeeded.
    pub resolved: Option<PathBuf>,
    /// Why the candidate was rejected.
    pub reason: LocalImageFailureReason,
}

impl LocalImageFailure {
    /// `raw (reason)`: the phrase quoted back to the model in a nudge.
    pub fn describe(&self) -> String {
        format!("{} ({})", self.raw, self.reason.as_str())
    }
}

/// Per-byte predicate: `regions[i]` is true when byte `i` of `text` sits
/// inside a code span or fenced block. A backtick toggles the state, so a
/// fenced block (three backticks) and an inline span (one) both open and
/// close with the same rule. One home for "is this inside code", shared by
/// every scanner that walks markdown-shaped reply text.
pub fn code_regions(text: &str) -> Vec<bool> {
    let mut regions = vec![false; text.len()];
    let mut in_code = false;
    for (i, byte) in text.bytes().enumerate() {
        regions[i] = in_code;
        if byte == b'`' {
            in_code = !in_code;
        }
    }
    regions
}

/// True when the reference is a Telegram media reference rather than a path.
/// These are resolved by Telegram against the `media` array of the rich
/// request that carries them, so a caller must never join one to the session
/// working directory and report a nonsense path back to the model.
pub(crate) fn is_telegram_media_ref(raw: &str) -> bool {
    let lower = raw.to_ascii_lowercase();
    lower.starts_with("tg://") || lower.starts_with("attach://")
}

/// True when the reference is a fetchable network URL rather than a local
/// path. `mailto:` and `ftp://` are deliberately absent: neither names a file
/// this host can ship, so a reference carrying one is left in the text as
/// written rather than reported as a missing file.
pub(crate) fn is_remote_url(raw: &str) -> bool {
    const SCHEMES: [&str; 3] = ["http://", "https://", "data:"];
    let lower = raw.to_ascii_lowercase();
    SCHEMES.iter().any(|scheme| lower.starts_with(scheme))
}

/// Parse a markdown reference starting at `start` (a char boundary where the
/// text begins with `[`). `bracket_len` is the length of the opening bracket:
/// 2 for an image (`![alt](target)`), 1 for a link (`[label](target)`); the
/// two forms differ in nothing else. Accepts the angle-bracket form
/// `(<target>)` that markdown requires when the path holds spaces, and an
/// optional `"title"` / `'title'` after the target. Returns
/// `(end_byte_exclusive, label, raw_target, title)`.
fn parse_markdown_ref(
    text: &str,
    start: usize,
    bracket_len: usize,
) -> Option<(usize, String, String, Option<String>)> {
    let open = match bracket_len {
        2 => "![",
        1 => "[",
        _ => return None,
    };
    debug_assert!(text[start..].starts_with(open));
    // `\![alt](path)` and `\[label](path)` are escaped literal text, not
    // references.
    if start > 0 && text[..start].ends_with('\\') {
        return None;
    }
    let label_end = text[start + bracket_len..].find(']')?;
    let paren = start + bracket_len + label_end + 1;
    if !text[paren..].starts_with('(') {
        return None;
    }
    let label = text[start + bracket_len..start + bracket_len + label_end].to_string();
    let cursor = skip_whitespace(text, paren + 1);
    let (target, mut after_target) = if text[cursor..].starts_with('<') {
        let close = text[cursor + 1..].find('>')?;
        let target = text[cursor + 1..cursor + 1 + close].to_string();
        (target, cursor + 1 + close + 1)
    } else {
        let mut end = cursor;
        while let Some(ch) = text[end..].chars().next() {
            if ch.is_whitespace() || ch == ')' {
                break;
            }
            end += ch.len_utf8();
        }
        (text[cursor..end].to_string(), end)
    };
    after_target = skip_whitespace(text, after_target);
    // The title is the only caption carrier markdown offers, so it is
    // captured here instead of being stepped over.
    let mut title: Option<String> = None;
    if let Some(quote) = text[after_target..].chars().next()
        && (quote == '"' || quote == '\'')
    {
        let close = text[after_target + 1..].find(quote)?;
        let parsed = text[after_target + 1..after_target + 1 + close].trim();
        if !parsed.is_empty() {
            title = Some(parsed.to_string());
        }
        after_target = skip_whitespace(text, after_target + 1 + close + 1);
    }
    if !text[after_target..].starts_with(')') || target.trim().is_empty() {
        return None;
    }
    Some((after_target + 1, label, target, title))
}

/// Byte offset of the first non-whitespace char at or after `from`.
fn skip_whitespace(text: &str, from: usize) -> usize {
    let mut cursor = from;
    while let Some(ch) = text[cursor..].chars().next() {
        if !ch.is_whitespace() {
            break;
        }
        cursor += ch.len_utf8();
    }
    cursor
}

/// True when `raw` begins a URI scheme (`mailto:`, `ftp:`, `tel:`, ...).
/// `http(s)://`, `data:` and `tg://`/`attach://` are covered by the caller's
/// earlier checks; this catches every OTHER scheme so a `mailto:` link is
/// left as written rather than joined to the session working directory and
/// reported as a missing file.
fn has_url_scheme(raw: &str) -> bool {
    let mut chars = raw.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    for ch in chars {
        if ch == ':' {
            return true;
        }
        if !(ch.is_ascii_alphanumeric() || ch == '+' || ch == '-' || ch == '.') {
            return false;
        }
    }
    false
}

/// Telegram's `sendDocument` ceiling. The one size gate that matters for the
/// family: a larger file is reported as a rejection reason BEFORE delivery is
/// attempted, instead of surfacing as an API refusal the model never saw
/// coming.
pub const TELEGRAM_DOCUMENT_MAX_BYTES: u64 = 50 * 1024 * 1024;

/// Validate a resolved local-file candidate: it must exist, be a regular
/// file, hold at least one byte, be openable, and fit the document size
/// ceiling. There is deliberately NO format gate: any bytes can ship as a
/// document, and rejecting an unrecognised format would delete a referenced
/// file from the reply and deliver nothing.
pub fn validate_local_file(path: &Path) -> Result<(), LocalImageFailureReason> {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(LocalImageFailureReason::NotFound);
        }
        Err(_) => return Err(LocalImageFailureReason::Unreadable),
    };
    if !meta.is_file() {
        return Err(LocalImageFailureReason::NotAFile);
    }
    if meta.len() == 0 {
        return Err(LocalImageFailureReason::Empty);
    }
    if meta.len() > TELEGRAM_DOCUMENT_MAX_BYTES {
        return Err(LocalImageFailureReason::TooLarge);
    }
    std::fs::File::open(path).map_err(|_| LocalImageFailureReason::Unreadable)?;
    Ok(())
}

/// What a markdown link target resolves to for the file family:
/// classification and validation in ONE step.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Resolution {
    /// A local file that exists, is a regular non-empty file, is openable and
    /// fits the document size ceiling.
    Local(PathBuf),
    /// A local path that failed validation, with the resolved path and why.
    Rejected {
        path: PathBuf,
        reason: LocalImageFailureReason,
    },
    /// Not a filesystem candidate at all: a remote URL, a Telegram media ref,
    /// a `mailto:`/`tel:`-style scheme, an in-page `#anchor`, or a relative
    /// path with no base directory to resolve it against. Left as written.
    Skip,
}

/// Resolve AND validate one link target for the file family.
///
/// ONE policy for both the scan ([`record_file_candidate`]) and any later
/// consumer, so the two can never disagree about which links were files.
/// `~/...` goes through the shared tilde expander, an absolute path is taken
/// as-is, and a relative path is joined to `base_dir` (the session working
/// directory).
fn resolve_file_target(target: &str, base_dir: Option<&Path>) -> Resolution {
    let trimmed = target.trim();
    // An in-page anchor (`#section`) is not a file.
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return Resolution::Skip;
    }
    // A link Telegram resolves itself (a real URL, a media ref), or one with
    // any other URI scheme (`mailto:`, `ftp:`, `tel:`), must not be joined to
    // the cwd and reported as a missing file.
    if is_remote_url(trimmed) || is_telegram_media_ref(trimmed) || has_url_scheme(trimmed) {
        return Resolution::Skip;
    }
    let expanded = crate::brain::tools::error::expand_tilde(trimmed);
    let path = if expanded.is_absolute() {
        expanded
    } else {
        match base_dir {
            Some(dir) => dir.join(expanded),
            // A relative target with no base directory may be ordinary prose
            // that merely looks like a link: leave it as written.
            None => return Resolution::Skip,
        }
    };
    match validate_local_file(&path) {
        Ok(()) => Resolution::Local(path),
        Err(reason) => Resolution::Rejected { path, reason },
    }
}

/// A resolved local file. The caption belongs to the link written around the
/// path, so the two travel as ONE value: parallel vectors would desync the
/// first time a candidate is dropped from one and not the other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFile {
    /// Absolute path to the file on disk.
    pub path: PathBuf,
    /// The markdown link label, `[label](target)`, shipped as the document
    /// caption. `None` when the label was empty.
    pub caption: Option<String>,
    /// Byte range in [`LocalFileScan::text`] occupied by this file's visible
    /// `📎 <label>` marker (#1918). The scanner records the span as it emits
    /// the marker, so a later pass can rewrite exactly that range: a label
    /// that also occurs elsewhere in the reply can never be mis-targeted.
    /// `None` for a value that did not come from a scan, so the sentinel is
    /// unrepresentable rather than a `(0, 0)` a reader must remember to test
    /// for.
    pub marker_span: Option<std::ops::Range<usize>>,
}

/// Result of scanning a reply for links to local files.
#[derive(Debug, Clone, Default)]
pub struct LocalFileScan {
    /// Reply text with every DELIVERED local-file link replaced by a
    /// visible `📎 <label>` marker (#1918). A remote link, a non-file
    /// scheme, a reference inside a code span and a REJECTED candidate are
    /// all left byte-identical.
    pub text: String,
    /// Resolved and validated local files, in order of appearance.
    pub attachments: Vec<LocalFile>,
    /// Rejected local candidates, in order of appearance.
    pub failures: Vec<LocalImageFailure>,
}

/// File one parsed link into the scan accumulators. Returns `true` when the
/// reference was consumed and replaced by the visible marker, which happens
/// ONLY for a resolved, validated file. A rejected candidate or a non-file
/// target returns `false`, so the link is copied through byte-identical: a
/// remote link Telegram resolves itself, and a missing one is a failure
/// report, never a silent strip.
fn record_file_candidate(
    raw: &str,
    label: &str,
    target: &str,
    base_dir: Option<&Path>,
    scan: &mut LocalFileScan,
) -> bool {
    match resolve_file_target(target, base_dir) {
        Resolution::Local(path) => {
            let caption = if label.trim().is_empty() {
                None
            } else {
                Some(label.to_string())
            };
            scan.attachments.push(LocalFile {
                path,
                caption,
                marker_span: None,
            });
            true
        }
        Resolution::Rejected { path, reason } => {
            scan.failures.push(LocalImageFailure {
                raw: raw.to_string(),
                resolved: Some(path),
                reason,
            });
            false
        }
        Resolution::Skip => false,
    }
}

/// The visible text left where a delivered file link was (#1918).
///
/// The link label is the natural marker: it is the words the author chose.
/// An empty label falls back to the file's own name so the marker is never
/// blank. Deliberately NOT a URL: a `t.me` message link exists only for
/// groups and channels, so a DM or a basic group has no form to offer,
/// while a marker needs no link form at all.
///
/// Crate-visible for the scanner tests; every production caller lives in
/// this module.
pub(crate) fn file_marker_text(label: &str, target: &str) -> String {
    let trimmed = label.trim();
    if !trimmed.is_empty() {
        trimmed.to_string()
    } else {
        std::path::Path::new(target)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string())
    }
}

/// Append the visible marker for a resolved file, `📎 <label>`, and return
/// the byte span it occupies in `out` (#1918). The single home of the
/// marker's shape.
fn push_file_marker(out: &mut String, label: &str, target: &str) -> std::ops::Range<usize> {
    let start = out.len();
    out.push_str("📎 ");
    out.push_str(&file_marker_text(label, target));
    start..out.len()
}

/// Scan a reply for markdown links to local files and hand back the text
/// with every DELIVERED link replaced by a visible marker, the validated
/// files, and the rejected candidates.
///
/// `base_dir` is the session working directory: a relative target resolves
/// against it. With no base directory a relative link stays verbatim while
/// `~`-prefixed and absolute targets still resolve.
///
/// Marker semantics differ from the image family on purpose. A resolved
/// file becomes an attachment AND the link that named it is replaced by a
/// visible `📎 <label>` marker (#1918), not deleted: the marker keeps the
/// file's name and the position it was referenced at, and it carries no
/// URL, so it renders in every chat kind. A REJECTED candidate stays in
/// the text byte-identical AND is reported as a failure, because a link
/// carries its own label and a silent strip would delete the reader's only
/// clue about what was referenced.
pub fn extract_local_files(text: &str, base_dir: Option<&Path>) -> LocalFileScan {
    let regions = code_regions(text);
    let mut scan = LocalFileScan {
        text: String::with_capacity(text.len()),
        ..LocalFileScan::default()
    };
    let mut i = 0;

    while i < text.len() {
        if !regions[i]
            && text[i..].starts_with('[')
            // `![alt](path)` is an image reference, not a file link: its `[`
            // is preceded by `!`, and copying the `!` first must not let the
            // link parser claim the span on the next iteration.
            && !text[..i].ends_with('!')
            && let Some((end, label, target, _title)) = parse_markdown_ref(text, i, 1)
            && record_file_candidate(&text[i..end], &label, &target, base_dir, &mut scan)
        {
            // #1918: the reference becomes a visible marker, not a hole.
            // The label is the marker text, and an empty label falls back
            // to the file's own name so the reader always has something to
            // anchor on. No URL is emitted, so the marker renders in every
            // chat kind.
            let span = push_file_marker(&mut scan.text, &label, &target);
            // The attachment pushed by `record_file_candidate` is the one
            // this marker belongs to: the scan is single-threaded and in
            // order.
            if let Some(record) = scan.attachments.last_mut() {
                record.marker_span = Some(span);
            }
            i = end;
            continue;
        }
        let ch = text[i..].chars().next().expect("i lies on a char boundary");
        scan.text.push(ch);
        i += ch.len_utf8();
    }

    // `trim()` strips leading whitespace, which shifts every recorded span
    // left by that many bytes. Rebase the spans BEFORE trimming so a
    // `marker_span` is always an index into the FINAL `scan.text`. Only the
    // LEADING run matters: a marker begins with `📎` and ends with a
    // non-whitespace label, so every span lies wholly inside
    // `trim_start()..trim_end()`, and any trailing whitespace sits after
    // the last marker and never moves an index.
    let lead = scan.text.len() - scan.text.trim_start().len();
    if lead > 0 {
        for record in &mut scan.attachments {
            if let Some(span) = &mut record.marker_span {
                span.start -= lead;
                span.end -= lead;
            }
        }
    }
    scan.text = scan.text.trim().to_string();
    scan
}

/// The honest user-visible line for files that could not be delivered.
/// `None` when there is nothing to report.
pub fn file_failure_notice(failures: &[LocalImageFailure]) -> Option<String> {
    if failures.is_empty() {
        return None;
    }
    let mut notice = String::from(
        "⚠️ File not attached: the reply referenced a file that could not be delivered:",
    );
    for failure in failures {
        notice.push_str("\n- ");
        notice.push_str(&failure.describe());
    }
    Some(notice)
}

/// Append [`file_failure_notice`] to a reply body, separated by a blank line.
/// The empty-body case is why this lives here rather than at each call site:
/// a reply whose ONLY content was a broken file reference leaves an empty
/// body, and a naive `format!` would deliver the notice with a stray blank
/// line or be skipped by a downstream emptiness check entirely. An empty body
/// becomes the notice.
pub fn append_file_failure_notice(body: &str, failures: &[LocalImageFailure]) -> String {
    match file_failure_notice(failures) {
        None => body.to_string(),
        Some(notice) => {
            if body.trim().is_empty() {
                notice
            } else {
                format!("{}\n\n{notice}", body.trim_end())
            }
        }
    }
}
