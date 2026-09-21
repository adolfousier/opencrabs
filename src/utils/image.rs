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
