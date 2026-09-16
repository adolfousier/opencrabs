//! Render the AST to Telegram HTML — the fallback path used when the rich
//! `sendRichMessage` API is unavailable or rejects a message.
//!
//! Telegram HTML has no table/heading/list tags, so structure is approximated:
//! headings become bold, lists become bullet/number/checkbox lines, and tables
//! become an aligned monospace grid inside `<pre>`. Inline styling is preserved
//! everywhere except inside table cells (preformatted text can't carry tags).

use super::ast::{Align, Block, Inline, List, MermaidResult, Table};
use super::mermaid;
use super::parse::parse_markdown;
use crate::channels::telegram::markdown::escape_html;

/// Render a block list to a Telegram-HTML string. Block-level elements are
/// separated by a blank line so paragraphs, headings, lists, and tables keep
/// their breathing room (the source markdown's paragraph breaks); list items
/// themselves stay single-spaced.
pub(super) fn render_html(blocks: &[Block]) -> String {
    render_html_inner(blocks, false)
}

/// Like [`render_html`] but wraps every paragraph in `<p>` tags and drops the
/// `\n\n` separator (the tags provide their own spacing). Used by the
/// chrome_rich path where Telegram's HTML renderer expects native paragraph
/// elements inside `<details>` blocks.
pub(super) fn render_html_p(blocks: &[Block]) -> String {
    render_html_inner(blocks, true)
}

fn render_html_inner(blocks: &[Block], wrap_p: bool) -> String {
    let sep = if wrap_p { "" } else { "\n\n" };
    blocks
        .iter()
        .map(|b| render_block(b, wrap_p))
        .collect::<Vec<_>>()
        .join(sep)
        .trim()
        .to_string()
}

fn render_block(block: &Block, wrap_p: bool) -> String {
    match block {
        Block::Heading { level, content } => {
            // No heading tags in Telegram HTML: bold, and italicize deeper
            // headings (level >= 3) so the hierarchy stays visible.
            let inner = render_inlines(content, wrap_p);
            let styled = if *level >= 3 {
                format!("<b><i>{inner}</i></b>")
            } else {
                format!("<b>{inner}</b>")
            };
            // wrap_p: a heading is a block too; without its own <p> two
            // consecutive headings merge into one line in the rich dialect.
            if wrap_p {
                format!("<p>{styled}</p>")
            } else {
                styled
            }
        }
        Block::Paragraph(content) => {
            let inner = render_inlines(content, wrap_p);
            if wrap_p {
                format!("<p>{inner}</p>")
            } else {
                inner
            }
        }
        Block::List(list) => render_list(list, 0, wrap_p),
        Block::Table(table) => render_table(table),
        Block::Code { lang, text } => match lang {
            Some(l) => format!(
                "<pre><code class=\"language-{}\">{}</code></pre>",
                escape_html(l),
                escape_html(text)
            ),
            None => format!("<pre><code>{}</code></pre>", escape_html(text)),
        },
        // Resolved mermaid fence (#1044): embed the rendered image, or degrade
        // to a legible failure block. Both HTML shapes are built in
        // super::mermaid so the escaping stays in one place.
        Block::Mermaid { source, result } => match result {
            MermaidResult::Image(url) => super::mermaid::image_html(url),
            // Locally-rendered PNG bytes are delivered via the multipart
            // markdown path, never through vector HTML `<img>` (Telegram
            // rejects it). This arm is a defensive fallback — degrade
            // legibly rather than leak raw binary.
            MermaidResult::ImageBytes(_) => super::mermaid::failure_html(
                "diagram rendered locally but could not be embedded in HTML",
                source,
            ),
            // #189: same split as the markdown path — a transient failure
            // offers the svg hatch (the response had already passed the
            // image check, so the render may exist server-side), while a
            // deterministic parse rejection does not.
            MermaidResult::Failed(err) => {
                super::mermaid::failure_html(err, source) + &super::mermaid::svg_link_html(source)
            }
            MermaidResult::ParseError(err) => super::mermaid::failure_html(err, source),
        },
        Block::Quote(inner) => format!(
            "<blockquote>{}</blockquote>",
            render_html_inner(inner, wrap_p)
        ),
        Block::Math(expr) => format!("<pre>{}</pre>", escape_html(expr)),
        Block::Divider => "──────────".to_string(),
        // Telegram HTML has no <details> — render as flat indented blocks
        // with a bold summary header so content is still visible.
        Block::Details {
            summary,
            blocks,
            open: _,
        } => {
            let summary_html = render_inlines(summary, wrap_p);
            let body = render_html_inner(blocks, wrap_p);
            format!(
                "<b>▸ {summary_html}</b>\n{}",
                body.lines()
                    .map(|l| format!("  {l}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        }
    }
}

/// Render a list to single-spaced lines. Nested child blocks (typically a
/// deeper list) are indented under their item. When `wrap_p` is set (the rich
/// `sendRichMessage` dialect, where a bare newline is just whitespace), every
/// line instead gets its own `<p>` so items keep their line breaks, and child
/// blocks are rendered in the same dialect.
fn render_list(list: &List, depth: usize, wrap_p: bool) -> String {
    let pad = "  ".repeat(depth);
    let mut lines = Vec::new();
    for (idx, item) in list.items.iter().enumerate() {
        let bullet = match item.task {
            Some(true) => "☑".to_string(),
            Some(false) => "☐".to_string(),
            None if list.ordered => format!("{}.", idx + 1),
            None => "•".to_string(),
        };
        let line = format!("{pad}{bullet} {}", render_inlines(&item.content, wrap_p));
        lines.push(if wrap_p {
            format!("<p>{line}</p>")
        } else {
            line
        });
        for child in &item.children {
            match child {
                Block::List(inner) => lines.push(render_list(inner, depth + 1, wrap_p)),
                other if wrap_p => lines.push(render_block(other, true)),
                other => lines.push(format!("{pad}  {}", render_block(other, false))),
            }
        }
    }
    lines.join(if wrap_p { "" } else { "\n" })
}

/// Width (monospace chars) up to which an aligned grid fits a typical phone.
/// Wider tables render as responsive cards / key-value lists instead, so they
/// never overflow the narrow Telegram or Web view (where a `<pre>` grid would
/// horizontal-scroll). The TUI sizes columns to the terminal; Telegram can't,
/// so this is how a "fits the view" table looks on messengers.
const NARROW_TABLE_WIDTH: usize = 40;

fn render_table(table: &Table) -> String {
    let cols = table.header.len();
    let header: Vec<String> = table.header.iter().map(|c| plain(c)).collect();
    let rows: Vec<Vec<String>> = table
        .rows
        .iter()
        .map(|r| r.iter().map(|c| plain(c)).collect())
        .collect();

    // Column widths from the widest plain-text cell in each column.
    let mut width = vec![0usize; cols];
    for (i, h) in header.iter().enumerate() {
        width[i] = width[i].max(h.chars().count());
    }
    for row in &rows {
        for (i, c) in row.iter().enumerate().take(cols) {
            width[i] = width[i].max(c.chars().count());
        }
    }

    // Width of one rendered grid line (columns + " | " separators). If it fits
    // a phone, keep the compact aligned grid; otherwise switch to a layout that
    // can't overflow: a key/value list for 2-column tables, cards for wider.
    let grid_width = width.iter().sum::<usize>() + cols.saturating_sub(1) * 3;
    if grid_width <= NARROW_TABLE_WIDTH {
        return render_grid(table, &header, &rows, &width);
    }
    if cols <= 2 {
        return render_key_value(table);
    }
    render_cards(table)
}

/// Compact aligned `<pre>` grid — used for tables narrow enough to fit a phone.
fn render_grid(table: &Table, header: &[String], rows: &[Vec<String>], width: &[usize]) -> String {
    let cols = width.len();
    let fmt = |cells: &[String]| -> String {
        width
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let cell = cells.get(i).map(String::as_str).unwrap_or("");
                let align = table.align.get(i).copied().unwrap_or(Align::None);
                let padded = pad_cell(cell, *w, align);
                if i + 1 < cols {
                    format!("{} | ", padded)
                } else {
                    padded
                }
            })
            .collect()
    };
    let sep: String = width
        .iter()
        .map(|w| "-".repeat(*w))
        .collect::<Vec<_>>()
        .join("-+-");
    let mut lines = vec![fmt(header), sep];
    for row in rows {
        lines.push(fmt(row));
    }
    format!("<pre>{}</pre>", escape_html(&lines.join("\n")))
}

/// One- or two-column table → a `key: value` list. The header row is dropped
/// because the columns are self-labelling (e.g. "Total commits: 52"). Wraps
/// naturally on any width; inline formatting is preserved (not inside `<pre>`).
fn render_key_value(table: &Table) -> String {
    // Cells come from single table lines and never contain a soft break, so
    // the wrap_p dialect flag is irrelevant here (false = no-op).
    let render = |inl: &[Inline]| render_inlines(inl, false);
    table
        .rows
        .iter()
        .map(|row| match row.get(1) {
            Some(val) => format!(
                "<b>{}</b>: {}",
                row.first().map(|c| render(c)).unwrap_or_default(),
                render(val)
            ),
            None => format!(
                "<b>{}</b>",
                row.first().map(|c| render(c)).unwrap_or_default()
            ),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 3+ column table → one card per row that never overflows: the first cell is
/// the bold title, the rest become "Header: value" lines. Inline formatting is
/// preserved.
fn render_cards(table: &Table) -> String {
    // Cells never contain soft breaks (single source lines): false is a no-op.
    let render = |inl: &[Inline]| render_inlines(inl, false);
    table
        .rows
        .iter()
        .map(|row| {
            let mut card = vec![format!(
                "<b>{}</b>",
                row.first().map(|c| render(c)).unwrap_or_default()
            )];
            for (i, cell) in row.iter().enumerate().skip(1) {
                let label = table.header.get(i).map(|h| plain(h)).unwrap_or_default();
                card.push(format!("{label}: {}", render(cell)));
            }
            card.join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn pad_cell(s: &str, width: usize, align: Align) -> String {
    let len = s.chars().count();
    if len >= width {
        return s.to_string();
    }
    let total = width - len;
    match align {
        Align::Right => format!("{}{}", " ".repeat(total), s),
        Align::Center => {
            let left = total / 2;
            format!("{}{}{}", " ".repeat(left), s, " ".repeat(total - left))
        }
        Align::Left | Align::None => format!("{}{}", s, " ".repeat(total)),
    }
}

fn render_inlines(inlines: &[Inline], wrap_p: bool) -> String {
    let mut s = String::new();
    for i in inlines {
        render_inline(i, &mut s, wrap_p);
    }
    s
}

fn render_inline(inline: &Inline, s: &mut String, wrap_p: bool) {
    match inline {
        // Soft line breaks inside a paragraph (#1142): the classic HTML
        // dialect renders a literal `\n` as a line break, so it passes
        // through; the rich dialect collapses a bare newline to whitespace,
        // so it becomes an explicit `<br>`. Code/math spans keep `\n` as-is
        // in both dialects (a newline inside inline code is data, not a
        // break).
        Inline::Text(t) => {
            let esc = escape_html(t);
            if wrap_p {
                s.push_str(&esc.replace('\n', "<br>"));
            } else {
                s.push_str(&esc);
            }
        }
        Inline::Bold(c) => wrap(s, "<b>", c, "</b>", wrap_p),
        Inline::Italic(c) => wrap(s, "<i>", c, "</i>", wrap_p),
        Inline::Underline(c) => wrap(s, "<u>", c, "</u>", wrap_p),
        Inline::Strike(c) => wrap(s, "<s>", c, "</s>", wrap_p),
        // Classic Telegram HTML has no <sub>; keep the content and drop the
        // tag so a downgraded rich message never shows its own markup.
        Inline::Sub(c) => {
            if wrap_p {
                wrap(s, "<sub>", c, "</sub>", wrap_p);
            } else {
                for x in c {
                    render_inline(x, s, wrap_p);
                }
            }
        }
        Inline::Code(t) | Inline::Math(t) => {
            s.push_str("<code>");
            s.push_str(&escape_html(t));
            s.push_str("</code>");
        }
        Inline::Link { content, url } => {
            s.push_str(&format!("<a href=\"{}\">", escape_html(url)));
            for c in content {
                render_inline(c, s, wrap_p);
            }
            s.push_str("</a>");
        }
    }
}

fn wrap(s: &mut String, open: &str, content: &[Inline], close: &str, wrap_p: bool) {
    s.push_str(open);
    for c in content {
        render_inline(c, s, wrap_p);
    }
    s.push_str(close);
}

/// Flatten inline content to plain text (used for table cells, which can't
/// carry styling tags inside `<pre>`).
fn plain(inlines: &[Inline]) -> String {
    let mut s = String::new();
    for i in inlines {
        plain_one(i, &mut s);
    }
    s
}

fn plain_one(inline: &Inline, s: &mut String) {
    match inline {
        Inline::Text(t) | Inline::Code(t) | Inline::Math(t) => s.push_str(t),
        Inline::Bold(c)
        | Inline::Italic(c)
        | Inline::Underline(c)
        | Inline::Strike(c)
        | Inline::Sub(c) => {
            for x in c {
                plain_one(x, s);
            }
        }
        Inline::Link { content, .. } => {
            for x in content {
                plain_one(x, s);
            }
        }
    }
}

/// Parse `text` and render it as Telegram HTML in one call (the fallback path).
pub(crate) fn markdown_to_html(text: &str) -> String {
    render_html(&parse_markdown(text))
}

/// Like [`markdown_to_html`] but wraps paragraphs in `<p>` tags. Used by the
/// chrome_rich path where Telegram expects native paragraph elements.
pub(crate) fn markdown_to_html_p(text: &str) -> String {
    render_html_p(&parse_markdown(text))
}

/// Parse `text`, resolve any mermaid fences to rendered images (or legible
/// failure blocks) via [`mermaid::resolve_blocks`], and render the result as
/// Telegram HTML. Async because mermaid pre-validation makes an HTTP call to
/// the renderer. This is the FALLBACK path, used when the primary
/// markdown+media send fails (e.g. a Bot API server < 10.2 without the
/// `media` field); the primary path keeps tables native via the markdown
/// dialect's `media` array (#1044).
///
/// Gated on [`mermaid::should_render_mermaid`]: when the feature is off or
/// the text has no mermaid fence, no HTTP is made and the output equals
/// [`markdown_to_html`]. Every chrome surface that can embed images renders
/// prose through this pair (or the `_p` variant) so a mermaid fence in plan
/// prose degrades exactly like one in a final reply (#1142).
pub(crate) async fn markdown_to_html_mermaid(text: &str) -> String {
    let blocks = parse_markdown(text);
    let resolved = if mermaid::should_render_mermaid(text) {
        mermaid::resolve_blocks(blocks).await
    } else {
        blocks
    };
    render_html(&resolved)
}

/// Like [`markdown_to_html_mermaid`] but wraps paragraphs in `<p>` tags (and
/// renders soft line breaks as `<br>`): the rich `sendRichMessage` HTML
/// dialect used by the plan card's prose bodies (#1142).
pub(crate) async fn markdown_to_html_mermaid_p(text: &str) -> String {
    let blocks = parse_markdown(text);
    let resolved = if mermaid::should_render_mermaid(text) {
        mermaid::resolve_blocks(blocks).await
    } else {
        blocks
    };
    render_html_p(&resolved)
}
