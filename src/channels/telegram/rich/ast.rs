//! Schema-independent AST for Telegram rich messages.
//!
//! Markdown text is parsed into this structure (see [`super::parse`]). The AST
//! is deliberately independent of any wire format so that:
//!
//! * the parser and its tests don't churn when Telegram's `InputRichMessage`
//!   field schema is finalized, and
//! * the same AST drives both the rich-first path (serialized to RichBlocks)
//!   and the HTML fallback ([`super::render_html`]).

/// A block-level element.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// ATX heading, `level` in `1..=6`.
    Heading { level: u8, content: Vec<Inline> },
    /// A run of inline content terminated by a blank line.
    Paragraph(Vec<Inline>),
    /// A bulleted or numbered list (items may carry task checkboxes and nested
    /// child blocks).
    List(List),
    /// A GitHub-flavored pipe table.
    Table(Table),
    /// A fenced code block with an optional language tag.
    Code { lang: Option<String>, text: String },
    /// A mermaid diagram fence, resolved to a rendered outcome before
    /// delivery (#1044). `source` is the original fence body, kept so the
    /// legible failure block can show what could not be rendered.
    #[cfg_attr(not(test), allow(dead_code))]
    Mermaid {
        source: String,
        result: MermaidResult,
    },
    /// A block quotation; may contain nested blocks.
    Quote(Vec<Block>),
    /// Display (block-level) math, e.g. `$$ ... $$`.
    Math(String),
    /// A horizontal rule / divider.
    Divider,
    /// A collapsible details block with a summary header.
    /// `<details>` / `<summary>` in markdown; renders as native collapsible
    /// in Telegram rich messages or flat indented blocks in HTML fallback.
    Details {
        summary: Vec<Inline>,
        blocks: Vec<Block>,
        open: bool,
    },
}

/// The outcome of rendering a mermaid diagram via the configured renderer.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq)]
pub enum MermaidResult {
    /// The renderer accepted the diagram; carries the image URL to embed.
    Image(String),
    /// Local (in-process) rendering produced PNG bytes ready for multipart
    /// upload to Telegram. Carries the raw encoded PNG.
    ImageBytes(Vec<u8>),
    /// The renderer rejected the diagram or was unreachable; carries a
    /// legible error note for the failure block. Transient in nature
    /// (timeout, unreachable, photo-box bust, server error) — the same
    /// source may render fine on a later attempt.
    Failed(String),
    /// The renderer rejected the diagram with a deterministic PARSE error
    /// (HTTP 4xx carrying the renderer's own error text). The source is
    /// broken as written; the note names the offending construct so the
    /// model can fix the fence and re-emit it (#37 regen nudge).
    ParseError(String),
}

/// A bulleted or numbered list.
#[derive(Debug, Clone, PartialEq)]
pub struct List {
    /// `true` for `1.`/`2.` ordered lists, `false` for `-`/`*`/`+` bullets.
    pub ordered: bool,
    pub items: Vec<ListItem>,
}

/// A single list item.
#[derive(Debug, Clone, PartialEq)]
pub struct ListItem {
    /// `None` for a normal item; `Some(checked)` for a `- [ ]` / `- [x]` task.
    pub task: Option<bool>,
    /// The item's own inline content (the text on the marker line).
    pub content: Vec<Inline>,
    /// Blocks indented under this item (typically a nested [`List`]).
    pub children: Vec<Block>,
}

/// Column alignment for a [`Table`], derived from the separator row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

/// A GitHub-flavored pipe table.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// Per-column alignment; same length as `header`.
    pub align: Vec<Align>,
    /// Header cells, each a run of inline content.
    pub header: Vec<Vec<Inline>>,
    /// Body rows; each row is a list of cells.
    pub rows: Vec<Vec<Vec<Inline>>>,
}

/// An inline-level element.
#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Text(String),
    Bold(Vec<Inline>),
    Italic(Vec<Inline>),
    Strike(Vec<Inline>),
    /// `<sub>` small text. The rich dialect renders it natively; classic
    /// Telegram HTML has no `<sub>`, so the renderer drops the tag and keeps
    /// the content. Parsed as a node rather than left as text so a lone
    /// `<sub>` in prose still escapes to visible characters.
    Sub(Vec<Inline>),
    /// `<u>` underline (classic Telegram HTML supports it natively; the rich
    /// dialect renders it natively too). Parsed as a node so producers of
    /// classic HTML don't escape into visible `&lt;u&gt;` (#106).
    Underline(Vec<Inline>),
    /// Inline `code` — content is literal, never re-parsed.
    Code(String),
    /// Inline `$math$` — content is literal, never re-parsed.
    Math(String),
    Link {
        content: Vec<Inline>,
        url: String,
    },
}

impl MermaidResult {
    #[allow(dead_code)]
    pub fn is_image(&self) -> bool {
        matches!(self, MermaidResult::Image(_) | MermaidResult::ImageBytes(_))
    }
}
