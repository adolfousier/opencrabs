//! Tests for TUI pane message rendering (unfocused-pane tail preservation).
//!
//! Extracted from an inline `#[cfg(test)]` block in
//! `src/tui/render/panes.rs`; project policy (CONTRIBUTING.md) requires
//! all tests under `src/tests/`.

use ratatui::text::Line;

use crate::tui::app::DisplayMessage;
use crate::tui::render::panes::render_simple_message;

fn assistant(content: &str) -> DisplayMessage {
    DisplayMessage {
        id: uuid::Uuid::nil(),
        role: "assistant".into(),
        content: content.into(),
        timestamp: Default::default(),
        token_count: None,
        cost: None,
        approval: None,
        approve_menu: None,
        details: None,
        expanded: false,
        expanded_full: false,
        tool_group: None,
        duration_secs: None,
    }
}

fn plain(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<Vec<_>>()
                .join("")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn an_unfocused_pane_keeps_the_tail_of_a_long_message() {
    // The removed cap cut at 500 raw bytes before parsing, so a PT-PT
    // answer lost its conclusion while rows sat blank below it (#1509).
    let body = "resumo com acentuação, conexões e passações longas. ".repeat(20);
    assert!(
        body.len() > 500,
        "fixture must exceed the removed byte cap to guard it: {} bytes",
        body.len()
    );
    let mut lines = Vec::new();
    render_simple_message(
        &mut lines,
        &assistant(&format!("{body}\nRESUMO: PRONTO")),
        70,
    );
    let text = plain(&lines);
    assert!(
        text.contains("PRONTO"),
        "the tail of the answer was cut: {text}"
    );
    assert!(
        !text.contains("..."),
        "a truncation sentinel leaked into the preview: {text}"
    );
}

#[test]
fn an_unfocused_pane_shows_what_the_parser_closed() {
    // Cutting raw bytes could stop inside a fenced block, leaving the
    // parser an unterminated fence. The line after the fence only
    // survives if the whole content reached the parser (#1509).
    let code = "linha_de_codigo_fonte_limpa_0123456789_\n".repeat(30);
    assert!(code.len() > 500, "fixture guard: {} bytes", code.len());
    let content = format!("```rust\n{code}```\nFECHAMENTO_OK");
    let mut lines = Vec::new();
    render_simple_message(&mut lines, &assistant(&content), 70);
    let text = plain(&lines);
    assert!(
        text.contains("FECHAMENTO_OK"),
        "content after the code block was cut: {text}"
    );
}
