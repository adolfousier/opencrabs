//! Rewrite markdown tables into Discord's own shape.
//!
//! Discord renders markdown — bold, headings, code blocks — but has no table
//! markup, so a pipe table reaches the channel as its own source in a
//! proportional font where no column lines up. Same gap Slack hit (#1016);
//! the parse here mirrors `slack::table_convert`, the render is Discord's.
//!
//! The table becomes what Telegram's rich renderer draws for the same input:
//! a monospace grid inside a fenced block, which Discord renders with real
//! column alignment. A table too wide for that falls back to Slack's
//! label-and-continuation shape with Discord bold — long cells wrap as
//! ordinary text instead of blowing out a grid column.
//!
//! Headings need nothing: Discord renders ATX headings natively.

use unicode_width::UnicodeWidthStr;

/// Total grid width past which the fenced block stops being readable and the
/// key-value fallback takes over.
const GRID_MAX_WIDTH: usize = 96;

/// A parsed pipe table.
struct Table {
    header: Vec<String>,
    rows: Vec<Vec<String>>,
}

/// Split a table line into trimmed cells, dropping the leading and trailing
/// empties that `|a|b|` produces.
fn cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').map(|c| strip_emphasis(c.trim())).collect()
}

/// The grid renders inside a fence, where Discord draws markdown source
/// literally — the `**bold**` and backticks the model writes into cells
/// would show as raw asterisks. Strip them here, at the single parse site.
/// The key-value fallback re-adds bold to labels at render time, outside
/// the fence where it actually renders.
fn strip_emphasis(cell: &str) -> String {
    cell.replace("**", "").replace('`', "")
}

/// Whether `line` is a separator row (`|---|:--:|`), which is what makes the
/// line above it a header rather than ordinary prose containing pipes.
fn is_separator(line: &str) -> bool {
    let c = cells(line);
    !c.is_empty()
        && c.iter().all(|cell| {
            !cell.is_empty()
                && cell.chars().all(|ch| ch == '-' || ch == ':' || ch == ' ')
                && cell.contains('-')
        })
}

/// Whether `line` could be a table row at all.
fn looks_like_row(line: &str) -> bool {
    line.trim().starts_with('|') && line.trim().len() > 1
}

/// Column widths across header and rows, measured in display cells.
fn col_widths(table: &Table) -> Vec<usize> {
    let mut w: Vec<usize> = table.header.iter().map(|c| c.width()).collect();
    for row in &table.rows {
        for (i, cell) in row.iter().enumerate() {
            if let Some(slot) = w.get_mut(i) {
                *slot = (*slot).max(cell.width());
            }
        }
    }
    w
}

/// Render the table as an aligned grid inside a fenced block.
fn render_grid(table: &Table) -> String {
    let w = col_widths(table);
    let pad = |row: &[String]| -> String {
        row.iter()
            .enumerate()
            .map(|(i, cell)| {
                // Ragged rows can exceed the header's column count; the clamp
                // keeps gap from underflowing there. Gap never hits zero for
                // well-formed tables (widths come from the same cells).
                let col = w.get(i).copied().unwrap_or(0);
                let gap = (col + 2).saturating_sub(cell.width());
                format!("{cell}{}", " ".repeat(gap))
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let rule = w
        .iter()
        .map(|x| "─".repeat(x + 2))
        .collect::<Vec<_>>()
        .join(" ");

    let mut out = vec!["```text".to_string(), pad(&table.header), rule];
    for row in &table.rows {
        out.push(pad(row));
    }
    out.push("```".to_string());
    out.join("\n")
}

/// Render the table as bold labels with `└` continuations — the shape a human
/// writes when the grid would be too wide to read.
fn render_key_value(table: &Table) -> String {
    let mut out = Vec::new();
    for row in &table.rows {
        let label = row.first().map(|s| s.as_str()).unwrap_or("");
        if label.is_empty() && row.iter().all(|c| c.is_empty()) {
            continue;
        }
        out.push(format!("**{label}**"));
        for (i, cell) in row.iter().enumerate().skip(1) {
            if cell.is_empty() {
                continue;
            }
            let head = table.header.get(i).map(|s| s.as_str()).unwrap_or("");
            if head.is_empty() {
                out.push(format!("└ {cell}"));
            } else {
                out.push(format!("└ {head}: {cell}"));
            }
        }
    }
    out.join("\n")
}

/// Render one table in Discord's shape.
fn render(table: &Table) -> String {
    let grid_width: usize = col_widths(table)
        .iter()
        .map(|x| x + 3)
        .sum::<usize>()
        .saturating_sub(1);
    if grid_width <= GRID_MAX_WIDTH {
        render_grid(table)
    } else {
        render_key_value(table)
    }
}

/// Rewrite every markdown table in `text`, leaving everything else alone.
///
/// Tables inside fenced code blocks are untouched: a fence is someone showing
/// the syntax, not asking for it to render.
pub(crate) fn tables_to_discord(text: &str) -> String {
    if !text.contains('|') {
        return text.to_string();
    }

    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    let mut in_fence = false;

    while i < lines.len() {
        let line = lines[i];
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            out.push(line.to_string());
            i += 1;
            continue;
        }
        if in_fence {
            out.push(line.to_string());
            i += 1;
            continue;
        }

        // A header row is only a header when a separator follows it.
        let has_table = looks_like_row(line)
            && i + 1 < lines.len()
            && looks_like_row(lines[i + 1])
            && is_separator(lines[i + 1]);

        if !has_table {
            out.push(line.to_string());
            i += 1;
            continue;
        }

        let header = cells(line);
        let mut rows = Vec::new();
        let mut j = i + 2;
        while j < lines.len() && looks_like_row(lines[j]) && !is_separator(lines[j]) {
            rows.push(cells(lines[j]));
            j += 1;
        }

        let rendered = render(&Table { header, rows });
        if !rendered.is_empty() {
            out.push(rendered);
        }
        i = j;
    }

    out.join("\n")
}
