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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

/// Captures location + filtered backtrace of the last panic so the
/// render loop can correlate a caught panic with its source. `catch_unwind`
/// only hands back the payload message, not the location — so we stash
/// both via the panic hook (which runs before `catch_unwind` swallows it)
/// and read them out after.
///
/// The `file:line:col` alone usually points at ratatui internals
/// (e.g. `ratatui-core/.../buffer.rs:250`) — useless without knowing
/// which of *our* widgets called in. The filtered backtrace picks the
/// first frame under `opencrabs::tui::` so the caller widget is named.
static LAST_PANIC_LOCATION: LazyLock<Mutex<Option<PanicInfo>>> = LazyLock::new(|| Mutex::new(None));

#[derive(Clone)]
struct PanicInfo {
    file: String,
    line: u32,
    column: u32,
    /// First OpenCrabs frame pulled out of the backtrace, if any.
    /// Format `module::fn at path:line` — typically the widget render call
    /// that fed ratatui an out-of-bounds coord.
    opencrabs_frame: Option<String>,
}

/// Walk a captured `Backtrace` and return the first frame whose symbol
/// name starts with `opencrabs::`. `Backtrace`'s `Display` impl renders
/// one frame per two lines:
///
/// ```text
///    N: symbol_name
///              at src/path.rs:LINE:COL
/// ```
///
/// We scan for any line containing `opencrabs::` (skipping the panic
/// hook itself), pull the symbol + `at ...` pair, and return them so
/// the render-panic log points at the actual widget that called
/// ratatui with an out-of-bounds coord, not ratatui's internal bounds
/// check.
fn first_opencrabs_frame(bt: &std::backtrace::Backtrace) -> Option<String> {
    let text = format!("{}", bt);
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed.contains("opencrabs::")
            && !trimmed.contains("first_opencrabs_frame")
            && !trimmed.contains("runner::")
        {
            let symbol = trimmed
                .trim_start_matches(|c: char| c.is_ascii_digit() || c == ':' || c == ' ')
                .to_string();
            let at = lines
                .peek()
                .filter(|l| l.trim().starts_with("at "))
                .map(|l| l.trim().to_string())
                .unwrap_or_default();
            return Some(if at.is_empty() {
                symbol
            } else {
                format!("{} {}", symbol, at)
            });
        }
    }
    None
}

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
    // Also stash the panic location so the render `catch_unwind` path can
    // log WHERE a caught render panic happened (the payload only carries
    // the message, not the source location).
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(loc) = info.location() {
            // Force-capture unconditionally — render panics are rare,
            // and RUST_BACKTRACE=1 is usually unset in TUI sessions.
            let bt = std::backtrace::Backtrace::force_capture();
            let opencrabs_frame = first_opencrabs_frame(&bt);
            if let Ok(mut slot) = LAST_PANIC_LOCATION.lock() {
                *slot = Some(PanicInfo {
                    file: loc.file().to_string(),
                    line: loc.line(),
                    column: loc.column(),
                    opencrabs_frame,
                });
            }
        }
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

    // Signal that the TUI owns stdout — suppress_stdio() will skip fd 1
    // redirection to avoid racing with ratatui's escape-sequence writes.
    crate::utils::fd_suppress::set_tui_active(true);

    // Force a full clear so stale content from a previous exec() restart
    // is wiped.
    terminal.clear()?;

    // Drain any stale terminal events (e.g. mouse events queued after a crash
    // where DisableMouseCapture never ran). Without this, queued escape
    // sequences leak into the input buffer as raw characters.
    while crossterm::event::poll(std::time::Duration::from_millis(10))? {
        let _ = crossterm::event::read();
    }

    // Fast sync init: decide mode and arm the header card. If we're going
    // to Chat, draw a first frame immediately so the user sees the header
    // card + empty chat + input *before* the blocking session load runs.
    // Onboarding skips the first frame so the wizard renders on first paint.
    let draw_first_frame = app.initialize_sync();
    if draw_first_frame {
        let app_ref: &mut App = &mut app;
        terminal.draw(move |f| render::render(f, app_ref))?;
    }

    // Now do the slow async init: load last session, sessions list, pane
    // preload, update check, DB integrity warnings. The header card stays
    // visible during this and vanishes on the 500ms timer from its arming
    // point inside `initialize_sync`.
    app.initialize().await?;

    // Start terminal event listener
    let event_sender = app.event_sender();
    EventHandler::start_terminal_listener(event_sender);

    // Run main loop
    let result = run_loop(&mut terminal, &mut app, &sigint_flag).await;

    // Restore terminal
    crate::utils::fd_suppress::set_tui_active(false);
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

    // Tracks the last applied mouse-capture state so we only call
    // Enable/DisableMouseCapture when the app's desired state changes.
    let mut mouse_capture_applied = true;

    loop {
        // Check SIGINT flag (redundant safety — the handler already exits,
        // but this catches the race where the flag is set before exit)
        if sigint_flag.load(Ordering::SeqCst) {
            break;
        }

        // Sync mouse-capture state if the user toggled it via F12.
        if app.mouse_capture_enabled != mouse_capture_applied {
            if app.mouse_capture_enabled {
                execute!(terminal.backend_mut(), EnableMouseCapture)?;
            } else {
                execute!(terminal.backend_mut(), DisableMouseCapture)?;
            }
            mouse_capture_applied = app.mouse_capture_enabled;
        }

        // Flush debounced session refresh from remote channels.
        // Only fires after 500ms of quiet to avoid blocking the loop with
        // rapid DB queries during multi-tool Telegram runs.
        if let Some((session_id, queued_at)) = app.pending_session_refresh
            && queued_at.elapsed() >= std::time::Duration::from_millis(500)
        {
            app.pending_session_refresh = None;
            if app.is_current_session(session_id)
                && !app.processing_sessions.contains(&session_id)
                && let Err(e) = app.load_session(session_id).await
            {
                tracing::warn!("Debounced session refresh failed: {}", e);
            }
        }

        // Consume pending resize (just clears the flag; ratatui's
        // autoresize inside draw() handles the actual buffer resize).
        app.pending_resize.take();

        // Wrap every frame in synchronized output (DEC private mode
        // 2026) so the terminal buffers all escape sequences and
        // displays the complete frame atomically.  This eliminates
        // flicker on resize because the clear + full redraw appear as
        // a single update to the terminal.
        let _ = execute!(
            terminal.backend_mut(),
            crossterm::terminal::BeginSynchronizedUpdate
        );

        // Render — wrap in catch_unwind so a render-time panic (e.g. a
        // ratatui buffer OOB from some edge-case layout) is caught, logged,
        // and the loop continues instead of crashing the whole TUI.
        // Reborrow terminal/app as fresh mutable references for this
        // iteration so moving them into the catch_unwind closure doesn't
        // consume the outer references permanently.
        let term_ref: &mut Terminal<CrosstermBackend<io::Stdout>> = &mut *terminal;
        let app_ref: &mut App = &mut *app;
        // Clear prior location before each draw so we only see what THIS
        // frame hit (panic hook overwrites the slot unconditionally, but
        // being explicit is cheap insurance).
        if let Ok(mut slot) = LAST_PANIC_LOCATION.lock() {
            *slot = None;
        }
        let draw_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            term_ref.draw(move |f| render::render(f, app_ref))
        }));
        match draw_result {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Err(e.into()),
            Err(panic_payload) => {
                let msg = if let Some(s) = panic_payload.downcast_ref::<&str>() {
                    (*s).to_string()
                } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "unknown render panic".to_string()
                };
                let (loc, caller) = LAST_PANIC_LOCATION
                    .lock()
                    .ok()
                    .and_then(|slot| slot.clone())
                    .map(|info| {
                        let loc = format!(" at {}:{}:{}", info.file, info.line, info.column);
                        let caller = info
                            .opencrabs_frame
                            .map(|f| format!(" — caller: {}", f))
                            .unwrap_or_default();
                        (loc, caller)
                    })
                    .unwrap_or_default();
                tracing::error!("[TUI] render panic caught{}{}: {}", loc, caller, msg);
                app.error_message = Some(format!("render panic{}{}: {}", loc, caller, msg));
                // Try to recover the terminal state for the next frame.
                let _ = terminal.clear();
            }
        }

        let _ = execute!(
            terminal.backend_mut(),
            crossterm::terminal::EndSynchronizedUpdate
        );

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
                    mouse_capture_applied = false;
                }
                TuiEvent::FocusGained => {
                    // Only re-enable mouse capture if the user hasn't
                    // explicitly turned it off via F12 (selection mode).
                    if app.mouse_capture_enabled {
                        execute!(terminal.backend_mut(), EnableMouseCapture)?;
                        mouse_capture_applied = true;
                    }
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
            // Coalesce Ticks, Scrolls, and streaming chunks to avoid redundant
            // re-renders. Streaming chunks are batched with a time budget so the
            // TUI stays responsive to keyboard/mouse input during long streams.
            let mut pending_scroll: i32 = 0;
            let drain_start = std::time::Instant::now();
            loop {
                match app.try_next_event() {
                    Some(TuiEvent::Tick) => continue,
                    Some(TuiEvent::MouseScroll(dir)) => {
                        pending_scroll += dir as i32;
                    }
                    Some(event) => {
                        let is_chunk = matches!(
                            event,
                            TuiEvent::ResponseChunk { .. } | TuiEvent::ReasoningChunk { .. }
                        );
                        if let Err(e) = app.handle_event(event).await {
                            app.error_message = Some(e.to_string());
                        }
                        // Batch streaming chunks for up to 30ms before yielding
                        // to render. This lets multiple chunks coalesce into one
                        // re-render instead of O(n) renders per second.
                        if is_chunk && drain_start.elapsed() >= std::time::Duration::from_millis(30)
                        {
                            break;
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
