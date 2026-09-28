//! Markdown-to-Telegram-HTML conversion and message splitting: pure text
//! transforms with no Bot API or state dependency.
//!
//! Moved VERBATIM out of handler.rs (#471 phase 1, pure decomposition —
//! only visibility widened to pub(crate) so the handler glob re-export
//! keeps every existing call site and test import stable).

use crate::config::Config;

/// Convert simple markdown to Telegram HTML: `**bold**`/`*bold*`, `` `code` ``,
/// and ```` ```fenced``` ```` code blocks.
///
/// CRITICAL: HTML-escape all text content (including inside code/bold) so a
/// literal `<...>` placeholder — e.g. `/rename <new title>` in /help — never
/// reaches Telegram as a tag. Unescaped, Telegram's HTML parser rejected the
/// whole message ("Unsupported start tag 'new'") and the reply silently
/// vanished. Only the tags we emit are real HTML.
///
/// Handles BOTH `**double**` and `*single*` asterisk bold (#650): the old
/// single-only pass turned `**ask**` into `<b></b>ask<b></b>` — an empty bold
/// pair around each `**` — which Telegram rejected, dropping the whole message
/// to plain text with the raw tags showing.
pub(crate) fn md_to_html(s: &str) -> String {
    fn esc(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }
    let mut out = String::with_capacity(s.len() + 16);
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '`' if chars.peek() == Some(&'`') => {
                chars.next(); // second backtick
                if chars.peek() == Some(&'`') {
                    // Fenced ``` block: drop the optional language on the fence
                    // line, then collect until the closing ```.
                    chars.next(); // third backtick
                    for ch in chars.by_ref() {
                        if ch == '\n' {
                            break;
                        }
                    }
                    let mut code = String::new();
                    while let Some(ch) = chars.next() {
                        if ch == '`' && chars.peek() == Some(&'`') {
                            chars.next();
                            if chars.peek() == Some(&'`') {
                                chars.next();
                                break;
                            }
                            code.push_str("``");
                        } else {
                            code.push(ch);
                        }
                    }
                    out.push_str("<pre><code>");
                    out.push_str(&esc(code.trim_end_matches('\n')));
                    out.push_str("</code></pre>");
                } else {
                    // Two backticks with nothing between — empty inline code.
                    out.push_str("<code></code>");
                }
            }
            '`' => {
                let code: String = chars.by_ref().take_while(|&ch| ch != '`').collect();
                out.push_str("<code>");
                out.push_str(&esc(&code));
                out.push_str("</code>");
            }
            '*' => {
                // **bold** (double) or *bold* (single).
                let double = chars.peek() == Some(&'*');
                if double {
                    chars.next();
                }
                let mut bold = String::new();
                while let Some(ch) = chars.next() {
                    if ch == '*' {
                        if !double {
                            break;
                        }
                        if chars.peek() == Some(&'*') {
                            chars.next();
                            break;
                        }
                        bold.push('*'); // lone `*` inside a `**` span → literal
                    } else {
                        bold.push(ch);
                    }
                }
                out.push_str("<b>");
                out.push_str(&esc(&bold));
                out.push_str("</b>");
            }
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// Convert markdown to Telegram HTML for channel command responses.
/// Routes through the full AST renderer (tables, lists, headings, code blocks)
/// when `channels.telegram.rich_messages` is enabled; falls back to the
/// lightweight `md_to_html` (bold + code only) otherwise.
pub(crate) fn command_md_to_html(s: &str) -> String {
    if Config::current().channels.telegram.rich_messages {
        super::rich::markdown_to_html(s)
    } else {
        md_to_html(s)
    }
}

pub(crate) fn strip_html_tags(html: &str) -> String {
    // Strip all HTML tags generically: anything between < and > is removed.
    // Handles <a href="...">, <u>, <s>, <blockquote>, and any other tag.
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;

    for c in html.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' && in_tag {
            in_tag = false;
        } else if !in_tag {
            result.push(c);
        }
    }

    // Unescape HTML entities
    result
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
}

/// Convert markdown to Telegram-safe HTML.
/// Handles: code blocks, inline code, bold, italic, underscore italic,
/// strikethrough, headers, links, list items, and plan-tool summary
/// blocks. Escapes HTML entities.
///
/// Plan blocks are emitted by the `plan` tool's `summary` op and
/// use markdown task lists that trigger rich detection. Example:
///
/// ```text
/// ## Plan: My Plan
/// **Status:** InProgress
/// - [x] First task
/// - [>] Second task
/// - [ ] Third task
/// **Progress:** 33.3%  ✅1 ❌0 ...
/// ```
///
/// When rich rendering is available, the function returns early via
/// `prefers_rich_render`. The legacy plan detector below handles
/// old-format text that doesn't trigger rich detection.
pub(crate) fn markdown_to_telegram_html(text: &str) -> String {
    // Re-expand a table the model collapsed onto one line (#690) so it is
    // detected below and rendered as a grid instead of raw pipes. Idempotent, so
    // callers that already reflowed (delivery) are unaffected; this catches the
    // other render paths (intermediates, resume).
    let reflowed = super::rich::reflow_collapsed_tables(text);
    let text = reflowed.as_str();
    // Messages containing a GitHub-flavored table or a task-list are rendered
    // through the rich AST: tables come out as aligned monospace grids (instead
    // of raw `| pipes |`) and task items as ☐/☑ (instead of literal `- [ ]`).
    // Everything else keeps the original line-based path (pinned by the sibling
    // render tests).
    if super::rich::prefers_rich_render(text) {
        return super::rich::markdown_to_html(text);
    }

    let mut result = String::with_capacity(text.len() + 256);
    let mut in_code_block = false;
    let mut in_plan_block = false;
    let mut code_lang;

    for line in text.lines() {
        // ── Plan-tool summary block: wrap in <pre> for monospace ──
        if !in_code_block && !in_plan_block && line.trim_start().starts_with("📊 Plan Summary") {
            result.push_str("<pre>");
            result.push_str(&escape_html(line));
            result.push('\n');
            in_plan_block = true;
            continue;
        }
        if in_plan_block {
            result.push_str(&escape_html(line));
            result.push('\n');
            // The `Success Rate:` line is always the last line of a
            // plan-summary block — see plan_tool.rs::execute summary
            // op. Close the <pre> and resume normal markdown
            // processing for any text that follows.
            if line.trim_start().starts_with("Success Rate:") {
                result.push_str("</pre>\n");
                in_plan_block = false;
            }
            continue;
        }

        if line.starts_with("```") {
            if in_code_block {
                result.push_str("</code></pre>\n");
                in_code_block = false;
            } else {
                code_lang = line.trim_start_matches('`').trim().to_string();
                if code_lang.is_empty() {
                    result.push_str("<pre><code>");
                } else {
                    result.push_str(&format!(
                        "<pre><code class=\"language-{}\">",
                        escape_html(&code_lang)
                    ));
                }
                in_code_block = true;
            }
            continue;
        }

        if in_code_block {
            result.push_str(&escape_html(line));
            result.push('\n');
            continue;
        }

        // Headers: # → bold. ATX semantics (1-6 hashes THEN a space), shared
        // with the rich parser's gate so the two can never disagree on the
        // same line (#1257). `starts_with('#')` alone swallowed the hashes of
        // every `#1234` issue reference in a receipt line and bolded the rest,
        // silently deleting the number the line existed to carry.
        let trimmed = line.trim_start();
        if crate::channels::telegram::rich::is_atx_heading(trimmed) {
            let content = trimmed.trim_start_matches('#').trim();
            let escaped = escape_html(content);
            result.push_str(&format!("<b>{}</b>\n", format_inline(&escaped)));
            continue;
        }

        // List items: - or * at start of line → bullet
        if (trimmed.starts_with("- ") || trimmed.starts_with("* ")) && trimmed.len() > 2 {
            let content = &trimmed[2..];
            let escaped = escape_html(content);
            // Preserve leading indent
            let indent = line.len() - trimmed.len();
            let spaces = &line[..indent];
            result.push_str(&format!(
                "{}• {}\n",
                escape_html(spaces),
                format_inline(&escaped)
            ));
            continue;
        }

        let escaped = escape_html(line);
        let formatted = format_inline(&escaped);
        result.push_str(&formatted);
        result.push('\n');
    }

    if in_code_block {
        result.push_str("</code></pre>\n");
    }
    if in_plan_block {
        // Plan summary truncated mid-stream (rare — the agent's
        // message got cut off before the Success Rate: footer).
        // Close the <pre> so Telegram doesn't reject the HTML.
        result.push_str("</pre>\n");
    }

    result.trim_end().to_string()
}

/// Escape HTML special characters
pub(crate) fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Decode the HTML entities [`escape_html`] emits, restoring the display text.
///
/// This is the exact inverse of [`escape_html`]: for every input `s`,
/// `unescape_html(escape_html(s)) == s`.
///
/// **Scope is deliberately narrow** — only the three entities `escape_html`
/// actually produces (`&lt;`, `&gt;`, `&amp;`). Decoding anything else would
/// break the round trip on labels that legitimately contain entity-looking
/// text (e.g. `&#65;`, `&quot;`) and would silently rewrite hand-authored
/// button labels written by other lanes.
///
/// **Order is load-bearing: `&amp;` MUST be decoded LAST.** `escape_html`
/// escapes `&` first, so a literal `&lt;` in the source is stored as
/// `&amp;lt;`. Decoding `&amp;` before `&lt;` would turn that back into `<`
/// instead of the literal `&lt;` the author wrote.
///
/// Callers measuring a label's display width must measure THIS text, not the
/// escaped form: entities inflate the character count (`&` costs 5, `<` and
/// `>` cost 4), so measuring escaped text rejects labels that fit.
pub(crate) fn unescape_html(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Apply inline formatting: `code`, **bold**, *italic*, _italic_, ~~strikethrough~~, [text](url)
pub(crate) fn format_inline(text: &str) -> String {
    // First pass: convert markdown links [text](url) → <a href="url">text</a>
    // Links are processed first because their syntax contains special chars
    let text = convert_links(text);

    let mut result = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '`' {
            if let Some(end) = chars[i + 1..].iter().position(|&c| c == '`') {
                let code: String = chars[i + 1..i + 1 + end].iter().collect();
                result.push_str(&format!("<code>{}</code>", code));
                i += end + 2;
                continue;
            }
        } else if chars[i] == '~' && i + 1 < chars.len() && chars[i + 1] == '~' {
            // ~~strikethrough~~
            if let Some(end) = find_closing_marker(&chars[i + 2..], &['~', '~']) {
                let inner: String = chars[i + 2..i + 2 + end].iter().collect();
                result.push_str(&format!("<s>{}</s>", inner));
                i += end + 4;
                continue;
            }
        } else if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            // **bold**
            if let Some(end) = find_closing_marker(&chars[i + 2..], &['*', '*']) {
                let inner: String = chars[i + 2..i + 2 + end].iter().collect();
                result.push_str(&format!("<b>{}</b>", inner));
                i += end + 4;
                continue;
            }
        } else if chars[i] == '_' && i + 1 < chars.len() && chars[i + 1] == '_' {
            // __bold__ (underscore bold)
            if let Some(end) = find_closing_marker(&chars[i + 2..], &['_', '_']) {
                let inner: String = chars[i + 2..i + 2 + end].iter().collect();
                result.push_str(&format!("<b>{}</b>", inner));
                i += end + 4;
                continue;
            }
        } else if chars[i] == '*' {
            // *italic*
            if let Some(end) = chars[i + 1..].iter().position(|&c| c == '*') {
                let inner: String = chars[i + 1..i + 1 + end].iter().collect();
                result.push_str(&format!("<i>{}</i>", inner));
                i += end + 2;
                continue;
            }
        } else if chars[i] == '_' {
            // _italic_ — only match if not part of a word (e.g. my_var should stay)
            let prev_alnum = i > 0 && chars[i - 1].is_alphanumeric();
            if !prev_alnum && let Some(end) = chars[i + 1..].iter().position(|&c| c == '_') {
                let next_alnum =
                    i + 1 + end + 1 < chars.len() && chars[i + 1 + end + 1].is_alphanumeric();
                if !next_alnum && end > 0 {
                    let inner: String = chars[i + 1..i + 1 + end].iter().collect();
                    result.push_str(&format!("<i>{}</i>", inner));
                    i += end + 2;
                    continue;
                }
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

/// Convert markdown links [text](url) to Telegram HTML <a> tags.
/// Operates on already-HTML-escaped text, so we must unescape the URL.
pub(crate) fn convert_links(text: &str) -> String {
    let mut result = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        result.push_str(&rest[..open]);
        let after_open = &rest[open + 1..];
        if let Some(close) = after_open.find("](") {
            let link_text = &after_open[..close];
            let after_paren = &after_open[close + 2..];
            if let Some(end_paren) = after_paren.find(')') {
                let url = &after_paren[..end_paren];
                // Unescape HTML entities first (escape_html ran before format_inline),
                // then re-escape for safe insertion into <a href="..."> attribute.
                let clean_url = url
                    .replace("&amp;", "&")
                    .replace("&lt;", "<")
                    .replace("&gt;", ">");
                let safe_url = escape_html(&clean_url);
                result.push_str(&format!("<a href=\"{}\">{}</a>", safe_url, link_text));
                rest = &after_paren[end_paren + 1..];
                continue;
            }
        }
        // Not a valid link, emit the '[' and continue
        result.push('[');
        rest = after_open;
    }
    result.push_str(rest);
    result
}

/// Find closing double-char marker (e.g. **) in a char slice
pub(crate) fn find_closing_marker(chars: &[char], marker: &[char]) -> Option<usize> {
    if marker.len() != 2 {
        return None;
    }
    (0..chars.len().saturating_sub(1)).find(|&i| chars[i] == marker[0] && chars[i + 1] == marker[1])
}

/// Split a message into chunks that fit Telegram's 4096 char limit
pub(crate) fn split_message(text: &str, max_len: usize) -> Vec<&str> {
    if text.chars().count() <= max_len {
        return vec![text];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.chars().count() <= max_len {
            chunks.push(remaining);
            break;
        }

        // Find the byte offset for max_len characters
        let byte_offset = remaining
            .char_indices()
            .nth(max_len)
            .map(|(i, _)| i)
            .unwrap_or(remaining.len());

        let chunk = &remaining[..byte_offset];

        // Try to break at a newline within the last 200 chars
        let break_at = chunk
            .rfind('\n')
            .filter(|&pos| {
                let chars_after_newline = chunk[pos + 1..].chars().count();
                chars_after_newline <= 200
            })
            .map(|pos| pos + 1)
            .unwrap_or(byte_offset);

        chunks.push(&remaining[..break_at]);
        remaining = &remaining[break_at..];
    }

    chunks
}
