//! TUI Runner
//!
//! Main event loop and terminal setup for the TUI.

use super::app::App;
use super::events::EventHandler;
use super::render;
use anyhow::Result;
use crossterm::{
    event::{
        DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
};
use std::io;

/// Run the TUI application
pub async fn run(mut app: App) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableMouseCapture,
        EnableFocusChange
    )?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Force a full clear so stale content from a previous exec() restart is wiped
    terminal.clear()?;

    // Initialize app
    app.initialize().await?;

    // Start terminal event listener
    let event_sender = app.event_sender();
    EventHandler::start_terminal_listener(event_sender);

    // Run main loop
    let result = run_loop(&mut terminal, &mut app).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableBracketedPaste,
        DisableMouseCapture,
        DisableFocusChange
    )?;
    terminal.show_cursor()?;

    result
}

/// Main event loop
async fn run_loop<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()>
where
    B::Error: Send + Sync + 'static,
{
    use super::events::TuiEvent;

    loop {
        // Render
        terminal.draw(|f| render::render(f, app))?;

        // Check for quit
        if app.should_quit {
            break;
        }

        // Wait for at least one event (with timeout for animation refresh)
        let event =
            tokio::time::timeout(tokio::time::Duration::from_millis(100), app.next_event()).await;

        if let Ok(Some(event)) = event {
            if let Err(e) = app.handle_event(event).await {
                app.error_message = Some(e.to_string());
            }

            // Drain all remaining queued events before re-rendering.
            // Coalesce Ticks and Scrolls to avoid redundant re-renders.
            let mut pending_scroll: i32 = 0;
            loop {
                match app.try_next_event() {
                    Some(TuiEvent::Tick) => continue,
                    Some(TuiEvent::MouseScroll(dir)) => {
                        pending_scroll += dir as i32;
                    }
                    Some(event) => {
                        if let Err(e) = app.handle_event(event).await {
                            app.error_message = Some(e.to_string());
                        }
                    }
                    None => break,
                }
            }
            // Apply coalesced scroll as a single operation
            if pending_scroll > 0 {
                app.scroll_offset = app.scroll_offset.saturating_add(pending_scroll as usize);
            } else if pending_scroll < 0 {
                app.scroll_offset = app
                    .scroll_offset
                    .saturating_sub(pending_scroll.unsigned_abs() as usize);
            }
        }
    }

    Ok(())
}
