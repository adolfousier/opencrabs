//! Block-level markdown parsing: the top-level dispatcher that turns lines of
//! markdown into [`Block`]s, delegating tables to [`super::table`], lists to
//! [`super::list`], and inline content to [`super::inline`].

use super::ast::Block;
use super::inline::parse_inlines;
use super::{list, table};

/// Parse a markdown document into a block list.
pub(crate) fn parse_markdown(input: &str) -> Vec<Block> {
    let lines: Vec<String> = input.lines().map(str::to_string).collect();
    parse_blocks(&lines)
}

/// Parse a slice of lines into blocks. Used recursively for blockquote bodies
/// and list-item children.
pub(super) fn parse_blocks(lines: &[String]) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let t = lines[i].trim();
        if t.is_empty() {
            i += 1;
            continue;
        }

        // Fenced code block.
        if let Some((fence_char, open_len, lang)) = parse_fence_open(t) {
            i += 1;
            let mut buf = Vec::new();
            while i < lines.len() && !is_fence_close(lines[i].trim(), fence_char, open_len) {
                buf.push(lines[i].clone());
                i += 1;
            }
            i += 1; // consume the closing fence (no-op past EOF)
            blocks.push(Block::Code {
                lang,
                text: buf.join("\n"),
            });
            continue;
        }

        // Pipe table (header row + separator row).
        if let Some((tbl, next)) = table::try_parse(lines, i) {
            blocks.push(Block::Table(tbl));
            i = next;
            continue;
        }

        // ATX heading.
        if let Some((level, content)) = heading(t) {
            blocks.push(Block::Heading {
                level,
                content: parse_inlines(content),
            });
            i += 1;
            continue;
        }

        // Horizontal rule.
        if is_divider(t) {
            blocks.push(Block::Divider);
            i += 1;
            continue;
        }

        // Block math: `$$` fence or a single `$$ ... $$` line.
        if t == "$$" {
            i += 1;
            let mut buf = Vec::new();
            while i < lines.len() && lines[i].trim() != "$$" {
                buf.push(lines[i].clone());
                i += 1;
            }
            i += 1; // consume the closing `$$`
            blocks.push(Block::Math(buf.join("\n")));
            continue;
        }
        if let Some(inner) = t.strip_prefix("$$").and_then(|s| s.strip_suffix("$$"))
            && !inner.is_empty()
        {
            blocks.push(Block::Math(inner.trim().to_string()));
            i += 1;
            continue;
        }

        // Blockquote.
        if t.starts_with('>') {
            let mut buf = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                let l = lines[i].trim_start();
                let stripped = l.strip_prefix('>').unwrap_or(l);
                buf.push(stripped.strip_prefix(' ').unwrap_or(stripped).to_string());
                i += 1;
            }
            blocks.push(Block::Quote(parse_blocks(&buf)));
            continue;
        }

        // List.
        if list::is_item(&lines[i]) {
            blocks.push(Block::List(list::parse_list(lines, &mut i)));
            continue;
        }

        // <details> / <summary> collapsible block.
        if is_details_open(t) {
            let open = t.contains(" open");
            i += 1; // consume <details> / <details open>
            // Optional <summary> on the next line(s).
            let mut summary_inlines = Vec::new();
            if i < lines.len() {
                let sl = lines[i].trim();
                if sl == "<summary>" {
                    // Multi-line summary: <summary> on own line
                    i += 1;
                    let mut summary_buf = Vec::new();
                    while i < lines.len() && lines[i].trim() != "</summary>" {
                        summary_buf.push(lines[i].trim().to_string());
                        i += 1;
                    }
                    if i < lines.len() {
                        i += 1; // consume </summary>
                    }
                    summary_inlines = parse_inlines(&summary_buf.join(" "));
                } else if let Some(rest) = sl.strip_prefix("<summary>")
                    && let Some(text) = rest.strip_suffix("</summary>")
                {
                    // Inline summary: <summary>text</summary> on one line
                    summary_inlines = parse_inlines(text.trim());
                    i += 1;
                }
            }
            // Collect body lines until matching </details>, respecting nesting.
            let mut body_buf = Vec::new();
            let mut depth = 1usize;
            while i < lines.len() {
                let bt = lines[i].trim();
                if is_details_open(bt) {
                    depth += 1;
                } else if bt == "</details>" {
                    depth -= 1;
                    if depth == 0 {
                        i += 1; // consume </details>
                        break;
                    }
                }
                body_buf.push(lines[i].clone());
                i += 1;
            }
            blocks.push(Block::Details {
                summary: summary_inlines,
                blocks: parse_blocks(&body_buf),
                open,
            });
            continue;
        }

        // Paragraph: gather soft-wrapped lines until a blank or a new block.
        // Lines are joined with `\n`, not the CommonMark space: chrome prose
        // is single-newline-wrapped, and the renderer — not the parser —
        // decides what a soft break means per dialect (literal `\n` in the
        // classic HTML dialect, `<br>` in the rich one). Joining with a
        // space here is what squashed multi-line prose into one visual line
        // (#1142).
        let mut buf = Vec::new();
        while i < lines.len() {
            if lines[i].trim().is_empty() || starts_block(lines, i) {
                break;
            }
            buf.push(lines[i].trim().to_string());
            i += 1;
        }
        blocks.push(Block::Paragraph(parse_inlines(&buf.join("\n"))));
    }

    blocks
}

/// Whether line `i` begins any block-level construct (used to terminate a
/// paragraph run).
fn starts_block(lines: &[String], i: usize) -> bool {
    let t = lines[i].trim();
    t.is_empty()
        || parse_fence_open(t).is_some()
        || heading(t).is_some()
        || is_divider(t)
        || t.starts_with('>')
        || t == "$$"
        || list::is_item(&lines[i])
        || table::try_parse(lines, i).is_some()
        || is_details_open(t)
}

/// Whether `t` is a `<details>` or `<details open>` opening tag.
fn is_details_open(t: &str) -> bool {
    t == "<details>" || t == "<details open>" || t.starts_with("<details ")
}

/// Parse an opening fence line (````lang` or `~~~lang`), returning `(delimiter_char, run_length, Option<lang>)`.
/// CommonMark requires at least 3 delimiter characters.
fn parse_fence_open(t: &str) -> Option<(char, usize, Option<String>)> {
    let first = t.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let count = t.chars().take_while(|&c| c == first).count();
    if count < 3 {
        return None;
    }
    let rest = t[count..].trim();
    // In backtick fences, the info string cannot contain backticks.
    if first == '`' && rest.contains('`') {
        return None;
    }
    let lang = if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    };
    Some((first, count, lang))
}

/// Whether line `t` closes a code fence that opened with delimiter `fence_char` of length `open_len`.
/// CommonMark closing fence: same delimiter character, run length >= open_len, no non-whitespace trailing characters.
fn is_fence_close(t: &str, fence_char: char, open_len: usize) -> bool {
    let count = t.chars().take_while(|&c| c == fence_char).count();
    if count < open_len {
        return false;
    }
    t[count..].trim().is_empty()
}

/// Parse an ATX heading, returning `(level, content)`.
fn heading(t: &str) -> Option<(u8, &str)> {
    let hashes = t.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&hashes) {
        let rest = &t[hashes..];
        if rest.is_empty() || rest.starts_with(' ') {
            return Some((hashes as u8, rest.trim()));
        }
    }
    None
}

/// Whether `t` is a horizontal rule (`---`, `***`, `___`, possibly spaced).
fn is_divider(t: &str) -> bool {
    let s: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    s.len() >= 3
        && (s.bytes().all(|b| b == b'-')
            || s.bytes().all(|b| b == b'*')
            || s.bytes().all(|b| b == b'_'))
}
