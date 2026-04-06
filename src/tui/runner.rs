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
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Force-restore terminal state. Safe to call from signal handlers and panic hooks.
fn force_restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        LeaveAlternateScreen,
        DisableBracketedPaste,
        DisableFocusChange,
        DisableMouseCapture
    );
    let _ = execute!(io::stdout(), crossterm::cursor::Show);
}

/// Run the TUI application
pub async fn run(mut app: App) -> Result<()> {
    // Install panic hook that restores terminal before printing the panic.
    // Without this, a panic leaves the terminal in raw mode with no cursor.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        force_restore_terminal();
        default_hook(info);
    }));

    // SIGINT (Ctrl+C) handler — forces terminal restoration and exits.
    // In raw mode, Ctrl+C is just byte 0x03 and crossterm may eat it if
    // stuck mid-escape-sequence. This handler bypasses the event loop.
    let sigint_flag = Arc::new(AtomicBool::new(false));
    let sigint_clone = sigint_flag.clone();
    tokio::spawn(async move {
        if let Ok(()) = tokio::signal::ctrl_c().await {
            sigint_clone.store(true, Ordering::SeqCst);
            force_restore_terminal();
            // Give a moment for terminal to restore, then hard-exit
            std::process::exit(130); // 128 + SIGINT(2)
        }
    });

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableFocusChange,
        EnableMouseCapture
    )?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Force a full clear so stale content from a previous exec() restart is wiped
    terminal.clear()?;

    // Drain any stale terminal events (e.g. mouse events queued after a crash
    // where DisableMouseCapture never ran). Without this, queued escape
    // sequences leak into the input buffer as raw characters.
    while crossterm::event::poll(std::time::Duration::from_millis(10))? {
        let _ = crossterm::event::read();
    }

    // Initialize app
    app.initialize().await?;

    // Start terminal event listener
    let event_sender = app.event_sender();
    EventHandler::start_terminal_listener(event_sender);

    // Run main loop
    let result = run_loop(&mut terminal, &mut app, &sigint_flag).await;

    // Restore terminal
    force_restore_terminal();

    result
}

/// Main event loop
async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    sigint_flag: &AtomicBool,
) -> Result<()> {
    use super::events::TuiEvent;

    loop {
        // Check SIGINT flag (redundant safety — the handler already exits,
        // but this catches the race where the flag is set before exit)
        if sigint_flag.load(Ordering::SeqCst) {
            break;
        }

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
            // Disable mouse capture when losing focus so the terminal stops
            // queuing SGR mouse sequences that pile up while unfocused.
            // Re-enable on focus regain and clear any garbage from input.
            match &event {
                TuiEvent::FocusLost => {
                    execute!(terminal.backend_mut(), DisableMouseCapture)?;
                }
                TuiEvent::FocusGained => {
                    execute!(terminal.backend_mut(), EnableMouseCapture)?;
                    // Clear any garbage that leaked into the input buffer
                    // while mouse capture was active in another tmux pane.
                    app.clear_escape_garbage();
                }
                _ => {}
            }

            if let Err(e) = app.handle_event(event).await {
                app.error_message = Some(e.to_string());
            }

            // Drain all remaining queued events before re-rendering.
            // Coalesce Ticks and Scrolls to avoid redundant re-renders.
            // Break on streaming chunks so each chunk triggers an immediate redraw.
            let mut pending_scroll: i32 = 0;
            loop {
                match app.try_next_event() {
                    Some(TuiEvent::Tick) => continue,
                    Some(TuiEvent::MouseScroll(dir)) => {
                        pending_scroll += dir as i32;
                    }
                    Some(event) => {
                        // Break on ResponseChunk so text appears immediately.
                        // ReasoningChunk is NOT broken on — reasoning can batch
                        // within the 100ms tick so it doesn't starve response text.
                        let is_response_chunk = matches!(event, TuiEvent::ResponseChunk { .. });
                        if let Err(e) = app.handle_event(event).await {
                            app.error_message = Some(e.to_string());
                        }
                        if is_response_chunk {
                            break; // Redraw immediately so each text chunk shows in real-time
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
