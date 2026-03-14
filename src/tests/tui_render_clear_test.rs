//! Tests to verify chat rendering uses Clear widget to prevent stale frame bleed.
//!
//! The core fix: `f.render_widget(Clear, area)` before `f.render_widget(chat, area)`
//! prevents reasoning/thinking text fragments from persisting across frames when
//! content shrinks between redraws.

use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph},
};

/// Simulate rendering a paragraph WITHOUT Clear — old content persists.
#[test]
fn without_clear_old_content_bleeds_through() {
    let backend = TestBackend::new(40, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    let area = Rect::new(0, 0, 40, 10);

    // Frame 1: render long content
    terminal
        .draw(|f| {
            let lines = vec![
                Line::from("Line 1: reasoning about the problem"),
                Line::from("Line 2: considering alternatives"),
                Line::from("Line 3: evaluating trade-offs here"),
                Line::from("Line 4: reaching a conclusion now"),
            ];
            let para = Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::NONE)
                    .padding(Padding::new(0, 0, 0, 0)),
            );
            f.render_widget(para, area);
        })
        .unwrap();

    // Capture frame 1 content
    let frame1 = terminal.backend().buffer().clone();
    let line4_frame1 = buffer_line_text(&frame1, 3, 40);
    assert!(
        line4_frame1.contains("reaching"),
        "Frame 1 should have line 4"
    );

    // Frame 2: render shorter content WITHOUT Clear
    terminal
        .draw(|f| {
            let lines = vec![Line::from("Line 1: short response")];
            let para = Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::NONE)
                    .padding(Padding::new(0, 0, 0, 0)),
            );
            // NO Clear widget — ratatui's TestBackend diff-renders, so old cells remain
            f.render_widget(para, area);
        })
        .unwrap();

    let frame2 = terminal.backend().buffer().clone();
    let line1_frame2 = buffer_line_text(&frame2, 0, 40);
    assert!(
        line1_frame2.contains("short response"),
        "Frame 2 line 1 should have new content"
    );
}

/// Simulate rendering a paragraph WITH Clear — old content is wiped.
#[test]
fn with_clear_old_content_is_wiped() {
    let backend = TestBackend::new(40, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    let area = Rect::new(0, 0, 40, 10);

    // Frame 1: render long reasoning content
    terminal
        .draw(|f| {
            let lines = vec![
                Line::from("Line 1: reasoning about the problem"),
                Line::from("Line 2: considering alternatives"),
                Line::from("Line 3: evaluating trade-offs here"),
                Line::from("Line 4: reaching a conclusion now"),
            ];
            let para = Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::NONE)
                    .padding(Padding::new(0, 0, 0, 0)),
            );
            f.render_widget(para, area);
        })
        .unwrap();

    // Frame 2: render shorter content WITH Clear first
    terminal
        .draw(|f| {
            let lines = vec![Line::from("Line 1: short response")];
            let para = Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::NONE)
                    .padding(Padding::new(0, 0, 0, 0)),
            );
            f.render_widget(Clear, area); // <-- THE FIX
            f.render_widget(para, area);
        })
        .unwrap();

    let frame2 = terminal.backend().buffer().clone();
    // Line 4 (index 3) should be blank now
    let line4_frame2 = buffer_line_text(&frame2, 3, 40);
    assert!(
        line4_frame2.trim().is_empty(),
        "With Clear, old line 4 should be blank but got: {:?}",
        line4_frame2
    );
    // Line 3 should also be blank
    let line3_frame2 = buffer_line_text(&frame2, 2, 40);
    assert!(
        line3_frame2.trim().is_empty(),
        "With Clear, old line 3 should be blank but got: {:?}",
        line3_frame2
    );
}

/// Verify reasoning text with rapidly changing length doesn't leave artifacts.
#[test]
fn reasoning_shrink_with_clear_leaves_no_artifacts() {
    let backend = TestBackend::new(60, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    let area = Rect::new(0, 0, 60, 8);

    // Simulate streaming: long reasoning grows then gets replaced by short response
    let reasoning_phases = vec![
        vec![
            "Thinking step 1...",
            "Thinking step 2...",
            "Thinking step 3...",
            "Thinking step 4...",
            "Thinking step 5...",
        ],
        vec!["Final answer: 42"],
    ];

    for phase in &reasoning_phases {
        terminal
            .draw(|f| {
                let lines: Vec<Line> = phase.iter().map(|s| Line::from(*s)).collect();
                let para = Paragraph::new(lines).block(
                    Block::default()
                        .borders(Borders::NONE)
                        .padding(Padding::new(0, 0, 0, 0)),
                );
                f.render_widget(Clear, area);
                f.render_widget(para, area);
            })
            .unwrap();
    }

    let final_buf = terminal.backend().buffer().clone();
    let line0 = buffer_line_text(&final_buf, 0, 60);
    assert!(line0.contains("Final answer"), "Should show final answer");

    // All other lines should be blank
    for row in 1..8 {
        let line = buffer_line_text(&final_buf, row, 60);
        assert!(
            line.trim().is_empty(),
            "Row {} should be blank after Clear but got: {:?}",
            row,
            line
        );
    }
}

/// Styled reasoning content is also properly cleared.
#[test]
fn styled_reasoning_cleared_properly() {
    let backend = TestBackend::new(50, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    let area = Rect::new(0, 0, 50, 6);

    let reasoning_style = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::ITALIC);

    // Frame 1: styled reasoning
    terminal
        .draw(|f| {
            let lines = vec![
                Line::from(Span::styled("Thinking...", reasoning_style)),
                Line::from(Span::styled(
                    "  analyzing the problem deeply",
                    reasoning_style,
                )),
                Line::from(Span::styled("  considering edge cases", reasoning_style)),
            ];
            let para = Paragraph::new(lines);
            f.render_widget(para, area);
        })
        .unwrap();

    // Frame 2: clear + short response
    terminal
        .draw(|f| {
            let lines = vec![Line::from("Done.")];
            let para = Paragraph::new(lines);
            f.render_widget(Clear, area);
            f.render_widget(para, area);
        })
        .unwrap();

    let buf = terminal.backend().buffer().clone();
    let line2 = buffer_line_text(&buf, 2, 50);
    assert!(
        line2.trim().is_empty(),
        "Styled reasoning line should be cleared: {:?}",
        line2
    );
}

/// Helper: extract text from a buffer row
fn buffer_line_text(buf: &Buffer, row: u16, width: u16) -> String {
    (0..width)
        .map(|col| {
            buf.cell((col, row))
                .map(|c| c.symbol().to_string())
                .unwrap_or_default()
        })
        .collect::<String>()
}
