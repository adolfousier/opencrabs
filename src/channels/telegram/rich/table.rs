//! GitHub-flavored pipe-table parsing.
//!
//! A table is a header row (`| a | b |`), a separator row of dashes with
//! optional alignment colons (`| :-- | --: |`), then zero or more body rows.
//! Parsing stops at the first line that is not a pipe row.

use super::ast::{Align, Inline, Table};
use super::inline::parse_inlines;
use regex::Regex;
use std::sync::LazyLock;

/// A row/header/separator boundary in a table the model collapsed onto one line:
/// the trailing `|` of one row meets the leading `|` of the next, giving an
/// empty gap between two pipes (`| |` or `||`). Matched non-greedily on a single
/// line (never crosses a real newline).
static COLLAPSED_ROW_BOUNDARY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\|[ \t]*\|").expect("collapsed-table boundary regex"));

/// Re-expand a table the model collapsed onto ONE line — header, separator, and
/// rows jammed together with no row-newlines — into a proper multi-line table so
/// [`try_parse`] can detect it and Telegram renders a grid instead of raw pipes
/// (#690). A line is treated as collapsed only when it carries BOTH a dash-only
/// separator cell AND ordinary content cells, a combination a well-formed
/// multi-line table never has on a single line — so prose, a lone separator row,
/// and already-expanded tables are left untouched. Idempotent.
pub(crate) fn reflow_collapsed_tables(text: &str) -> String {
    if !text.contains('|') {
        return text.to_string();
    }
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        if is_collapsed_table_line(line) {
            // #132: a prose label before the first pipe (e.g. "Состояние
            // данных: | Что | Статус |...") would stay glued to the split
            // header row — unparseable as a table AND rendered inside the
            // header cell. Detach everything before the first '|' onto its
            // own line so the table block starts clean.
            let pipe = line.find('|').unwrap_or(0);
            let (prefix, rest) = line.split_at(pipe);
            if !prefix.trim().is_empty() {
                out.push(prefix.trim_end().to_string());
            }
            out.push(
                COLLAPSED_ROW_BOUNDARY
                    .replace_all(rest, "|\n|")
                    .into_owned(),
            );
        } else {
            out.push(line.to_string());
        }
    }
    out.join("\n")
}

/// Whether a single line is a collapsed table: it has a dash-only separator cell
/// (`----`) AND at least one content cell on the same line. A proper multi-line
/// table has the separator alone on its own line, so this never fires on one.
fn is_collapsed_table_line(line: &str) -> bool {
    let mut has_dash = false;
    let mut has_content = false;
    for cell in line.split('|') {
        let c = cell.trim();
        if c.is_empty() {
            continue;
        }
        let core = c.trim_start_matches(':').trim_end_matches(':');
        if core.len() >= 2 && core.chars().all(|ch| ch == '-') {
            has_dash = true;
        } else {
            has_content = true;
        }
        if has_dash && has_content {
            return true;
        }
    }
    false
}

/// Insert one blank line before any table block whose preceding line is
/// non-blank (#95). Telegram's rich-markdown parser accepts a GFM table only
/// as a standalone block: a table that directly follows a text line renders
/// as raw pipes (probe matrix A/B/C/D — blank line present = rendered,
/// abutting = raw). Detection reuses [`try_parse`], so exactly the blocks the
/// AST parser accepts get normalized — prose with stray pipes is never
/// touched. Code fences (``` and ~~~) are never mutated, the pass is
/// idempotent, and pipe-free input returns unchanged.
pub(crate) fn ensure_blank_line_before_tables(text: &str) -> String {
    if !text.contains('|') {
        return text.to_string();
    }
    let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 1);
    let mut in_fence = false;
    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            out.push(line.clone());
            i += 1;
            continue;
        }
        if !in_fence && let Some((_, next)) = try_parse(&lines, i) {
            let prev_is_text = out.last().is_some_and(|prev| !prev.trim().is_empty());
            if prev_is_text {
                out.push(String::new());
            }
            while i < next {
                out.push(lines[i].clone());
                i += 1;
            }
            continue;
        }
        out.push(line.clone());
        i += 1;
    }
    let mut result = out.join("\n");
    if text.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// If a table begins at `lines[start]` (a pipe row immediately followed by a
/// separator row), parse it and return the table plus the index just past it.
pub(super) fn try_parse(lines: &[String], start: usize) -> Option<(Table, usize)> {
    let header_line = lines.get(start)?;
    let sep_line = lines.get(start + 1)?;
    if !looks_like_row(header_line) || !is_separator(sep_line) {
        return None;
    }

    let header: Vec<Vec<Inline>> = split_cells(header_line)
        .into_iter()
        .map(|c| parse_inlines(c.trim()))
        .collect();
    let align = parse_alignment(sep_line, header.len());

    let mut rows = Vec::new();
    let mut i = start + 2;
    while i < lines.len() && looks_like_row(&lines[i]) {
        let row: Vec<Vec<Inline>> = split_cells(&lines[i])
            .into_iter()
            .map(|c| parse_inlines(c.trim()))
            .collect();
        rows.push(row);
        i += 1;
    }

    Some((
        Table {
            align,
            header,
            rows,
        },
        i,
    ))
}

/// A line that could be a table row: contains a pipe and isn't blank.
fn looks_like_row(line: &str) -> bool {
    let t = line.trim();
    t.contains('|') && !t.is_empty()
}

/// A separator row: every cell is dashes with optional leading/trailing colon.
fn is_separator(line: &str) -> bool {
    let cells = split_cells(line);
    if cells.is_empty() {
        return false;
    }
    cells.iter().all(|c| {
        let c = c.trim();
        let core = c.trim_start_matches(':').trim_end_matches(':');
        !core.is_empty() && core.chars().all(|ch| ch == '-')
    })
}

/// Split a pipe row into trimmed cell strings, dropping the empty cells that
/// flank a row written with outer pipes (`| a | b |`).
fn split_cells(line: &str) -> Vec<&str> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').collect()
}

/// Derive per-column [`Align`] from the separator row, padded/truncated to
/// `cols` so it always matches the header width.
fn parse_alignment(sep: &str, cols: usize) -> Vec<Align> {
    let mut align: Vec<Align> = split_cells(sep)
        .into_iter()
        .map(|c| {
            let c = c.trim();
            let left = c.starts_with(':');
            let right = c.ends_with(':');
            match (left, right) {
                (true, true) => Align::Center,
                (true, false) => Align::Left,
                (false, true) => Align::Right,
                (false, false) => Align::None,
            }
        })
        .collect();
    align.resize(cols, Align::None);
    align
}

/// Single canonical table-normalization entry for the rich plane (#132):
/// balance unclosed / runaway code fences (#240), expand collapsed one-line
/// tables first (so [`try_parse`] can see them), infer missing table separators
/// (#239), then insert the blank line Telegram's rich parser demands before a
/// table block (#95). Every rich-build entry point and the structure-detection
/// gate call THIS — never the passes individually — so gate and renderer always
/// agree on the same text and a new send path inherits all fixes (#690, #980,
/// #1085 whack-a-mole retired). All passes are idempotent and fence-safe;
/// pipe-free input returns unchanged.
///
/// Also shields bare leading hashes (e.g. `#174`) so Telegram's rich parser
/// doesn't promote them into headings without CommonMark's required trailing space (#193).
pub(crate) fn normalize_tables(text: &str) -> String {
/// balance unclosed / runaway code fences (#240), expand collapsed one-line
/// tables first (so [`try_parse`] can see them), infer missing table separators
/// (#239), then insert the blank line Telegram's rich parser demands before a
/// table block (#95). Every rich-build entry point and the structure-detection
/// gate call THIS — never the passes individually — so gate and renderer always
/// agree on the same text and a new send path inherits all fixes (#690, #980,
/// #1085 whack-a-mole retired). All passes are idempotent and fence-safe;
/// pipe-free input returns unchanged.
}

/// Balance unclosed or runaway code fences in markdown text (#240).
///
/// LLM responses occasionally emit an odd number of ```` ``` ```` or `~~~` delimiters,
/// or an unclosed bare code fence immediately preceding top-level Markdown structure
/// (such as section headings `## ` or tables `|---|`). Without balancing, Telegram's
/// server-side parser and the local fallback parser swallow all subsequent text into
/// a giant monospaced code block.
///
/// Rules:
/// 1. Track open code fence delimiter (`'` or `~`) and minimum run length (>=3).
/// 2. For bare/unlabeled code fences (no language tag), if a top-level ATX heading
///    (e.g. `## `) or GFM table separator row (`|---|`) is encountered while `in_fence`
///    is true, the runaway fence is closed immediately prior to that line. Explicitly
///    labeled fences (e.g. ` ```rust `, ` ```python `) remain open across `#` comments.
/// 3. If a code fence remains unclosed at EOF, automatically append the matching closing
///    delimiter on its own line.
pub(crate) fn balance_code_fences(text: &str) -> String {
    if !text.contains("```") && !text.contains("~~~") {
        return text.to_string();
    }

    let mut out: Vec<String> = Vec::new();
    let mut in_fence = false;
    let mut fence_char = ' ';
    let mut fence_len = 0;
    let mut fence_has_lang = false;

    let lines: Vec<&str> = text.split('\n').collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();

        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let ch = trimmed.chars().next().unwrap();
            let count = trimmed.chars().take_while(|&c| c == ch).count();
            if count >= 3 {
                let rest = trimmed[count..].trim();
                let has_backtick_in_info = ch == '`' && rest.contains('`');

                if !has_backtick_in_info {
                    if !in_fence {
                        in_fence = true;
                        fence_char = ch;
                        fence_len = count;
                        fence_has_lang = !rest.is_empty();
                        out.push(line.to_string());
                        i += 1;
                        continue;
                    } else if ch == fence_char && count >= fence_len && rest.is_empty() {
                        in_fence = false;
                        fence_has_lang = false;
                        out.push(line.to_string());
                        i += 1;
                        continue;
                    }
                }
            }
        }

        // Check for runaway bare fence: if inside an unlabeled fence and we hit a top-level
        // ATX heading or a GFM table delimiter row, close the fence early before this line.
        if in_fence && !fence_has_lang {
            let is_heading = trimmed.starts_with('#') && super::detect::is_atx_heading(trimmed);
            let is_table_sep = is_separator(trimmed);
            let is_table_start = if i + 1 < lines.len() {
                looks_like_row(trimmed) && is_separator(lines[i + 1].trim_start())
            } else {
                false
            };

            if is_heading || is_table_sep || is_table_start {
                out.push(fence_char.to_string().repeat(fence_len));
                in_fence = false;
                out.push(line.to_string());
                i += 1;
                continue;
            }
        }

        out.push(line.to_string());
        i += 1;
    }

    if in_fence {
        out.push(fence_char.to_string().repeat(fence_len));
    }

    let mut result = out.join("\n");
    if text.ends_with('\n') && !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Escape bare leading `#` (not followed by space, or not a valid ATX heading)
/// with a backslash (`\#`) so Telegram's native rich parser does not promote
/// lines like `#174` into `<h1>` headers (#193, adolfousier/opencrabs#1257).
///
/// Leaves code fences (``` and ~~~) and valid ATX headings untouched.
pub(crate) fn shield_bare_leading_hashes(text: &str) -> String {
    if !text.contains('#') {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len() + 16);
    let mut in_fence = false;
    let mut fence_char = ' ';
    let mut fence_len = 0;

    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }

        let trimmed = line.trim_start();

        // Check for code fence start/end
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let ch = trimmed.chars().next().unwrap();
            let count = trimmed.chars().take_while(|&c| c == ch).count();
            if !in_fence {
                in_fence = true;
                fence_char = ch;
                fence_len = count;
                out.push_str(line);
                continue;
            } else if ch == fence_char && count >= fence_len {
                in_fence = false;
                out.push_str(line);
                continue;
            }
        }

        if in_fence {
            out.push_str(line);
            continue;
        }

        // Outside fence: check if line starts with bare # that is NOT an ATX heading
        // and not already escaped.
        if trimmed.starts_with('#') && !super::is_atx_heading(trimmed) {
            let indent_len = line.len() - trimmed.len();
            out.push_str(&line[..indent_len]);
            out.push('\\');
            out.push_str(trimmed);
        } else {
            out.push_str(line);
        }
    }

    out
}
