//! Log viewer state for Mission Control (#1528).
//!
//! Logs were write-only from the app's side: `logging/logger.rs` writes a
//! daily file and nothing ever read one back, so debugging meant leaving the
//! TUI to grep — at exactly the moment you wanted the context the TUI had.
//!
//! Everything here is pure except [`LogViewerState::open`] and
//! [`LogViewerState::load`], which touch the disk. That split is deliberate:
//! the filtering and scrolling rules are what a reader actually depends on, so
//! they are testable against fixture text rather than against a real log.

use std::path::PathBuf;

use crate::logging::reader::{self, LogEntry, LogLevel};

/// Rows the viewer scrolls by on a page key. One screen would be ideal but the
/// height is not known outside the renderer, so this matches the step the rest
/// of the TUI uses.
const PAGE_ROWS: usize = 20;

/// One open log file plus the filters applied to it.
#[derive(Debug, Clone)]
pub struct LogViewerState {
    /// Files available to page through, oldest first.
    pub files: Vec<PathBuf>,
    /// Index into `files` of the file on screen.
    pub file_index: usize,
    /// Entries parsed from the current file's tail.
    pub entries: Vec<LogEntry>,
    /// Minimum severity to show. Defaults to `Debug` rather than `Trace`
    /// because `Trace` is off in every shipped config, so defaulting to it
    /// would promise a level the file never contains.
    pub min_level: LogLevel,
    /// Substring filter across target, message and continuation lines.
    pub search: String,
    /// Whether keystrokes are going into the search box.
    pub search_active: bool,
    /// Rows scrolled past the top.
    pub scroll: usize,
    /// Written by the renderer so the key handler can clamp scrolling to what
    /// actually fits, the same way the help screen does (#1527).
    pub viewport_rows: usize,
    /// Set when the file could not be read, so the viewer can say why instead
    /// of rendering as though the log were empty.
    pub error: Option<String>,
}

impl Default for LogViewerState {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            file_index: 0,
            entries: Vec::new(),
            min_level: LogLevel::Debug,
            search: String::new(),
            search_active: false,
            scroll: 0,
            viewport_rows: 0,
            error: None,
        }
    }
}

impl LogViewerState {
    /// Open on the newest log file.
    ///
    /// Newest because the thing you just did is at the end of it: opening on
    /// the oldest file would make the common case a navigation exercise.
    pub fn open() -> Self {
        let files = reader::available_logs(&reader::log_dir());
        let mut state = Self {
            file_index: files.len().saturating_sub(1),
            files,
            ..Default::default()
        };
        state.load();
        state
    }

    /// Read and parse the current file's tail.
    ///
    /// A read failure is recorded rather than propagated: the viewer is a
    /// debugging aid, and failing to open one day's file should not close the
    /// screen you are debugging from.
    pub fn load(&mut self) {
        self.entries.clear();
        self.error = None;
        self.scroll = 0;
        let Some(path) = self.files.get(self.file_index) else {
            self.error = Some("No log files found. Is file logging enabled?".to_string());
            return;
        };
        match reader::tail_entries(path, reader::TAIL_BYTES) {
            Ok(entries) => self.entries = entries,
            Err(e) => self.error = Some(format!("Cannot read {}: {e}", path.display())),
        }
    }

    /// Name of the file on screen, for the status bar.
    pub fn file_name(&self) -> String {
        self.files
            .get(self.file_index)
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("(no file)")
            .to_string()
    }

    /// Entries passing both filters, in file order.
    ///
    /// A continuation line is carried inside its entry rather than being a row
    /// of its own, which is what keeps a level filter from eating the body of
    /// an error while leaving its first line behind.
    pub fn visible(&self) -> Vec<&LogEntry> {
        self.entries
            .iter()
            .filter(|e| e.level.at_least(self.min_level) && e.matches(&self.search))
            .collect()
    }

    /// Total rendered rows of the visible entries, counting continuations.
    pub fn content_rows(&self) -> usize {
        self.visible().iter().map(|e| e.lines()).sum()
    }

    /// Largest scroll offset that still leaves content on screen.
    pub fn max_scroll(&self) -> usize {
        self.content_rows().saturating_sub(self.viewport_rows)
    }

    /// Jump to the end, which is where a just-reproduced problem lives.
    pub fn scroll_to_end(&mut self) {
        self.scroll = self.max_scroll();
    }

    pub fn scroll_up(&mut self, rows: usize) {
        self.scroll = self.scroll.saturating_sub(rows);
    }

    pub fn scroll_down(&mut self, rows: usize) {
        self.scroll = self.scroll.saturating_add(rows).min(self.max_scroll());
    }

    pub fn page_up(&mut self) {
        self.scroll_up(PAGE_ROWS);
    }

    pub fn page_down(&mut self) {
        self.scroll_down(PAGE_ROWS);
    }

    /// Change the level filter. Scroll resets because the row the offset
    /// pointed at may no longer be on screen, or may no longer exist.
    pub fn set_level(&mut self, level: LogLevel) {
        self.min_level = level;
        self.scroll = 0;
    }

    /// Move to an adjacent day's file. Stops at the ends rather than wrapping:
    /// wrapping from the newest file to the oldest would read as a jump
    /// backwards in time with no indication it happened.
    pub fn step_file(&mut self, delta: isize) {
        if self.files.is_empty() {
            return;
        }
        let last = self.files.len() - 1;
        let next = match delta.cmp(&0) {
            std::cmp::Ordering::Less => self.file_index.saturating_sub(delta.unsigned_abs()),
            std::cmp::Ordering::Greater => (self.file_index + delta as usize).min(last),
            std::cmp::Ordering::Equal => self.file_index,
        };
        if next != self.file_index {
            self.file_index = next;
            self.load();
            self.scroll_to_end();
        }
    }

    /// Append to the search filter.
    pub fn push_search(&mut self, c: char) {
        self.search.push(c);
        self.scroll = 0;
    }

    /// Remove the last search character; closes the box when already empty so
    /// backspacing out of a filter does not strand the user in a prompt.
    pub fn pop_search(&mut self) {
        if self.search.is_empty() {
            self.search_active = false;
        } else {
            self.search.pop();
        }
        self.scroll = 0;
    }

    /// Clear the filter and leave the search box.
    pub fn clear_search(&mut self) {
        self.search.clear();
        self.search_active = false;
        self.scroll = 0;
    }
}

/// Map a keystroke to a level filter, or `None` when it is not a level key.
///
/// Both digits and initials are accepted because neither is obviously right:
/// `1`-`4` match the analytics window keys next door, and `e`/`w`/`i`/`d` are
/// what someone who knows log levels would try first.
pub fn level_for_key(c: char) -> Option<LogLevel> {
    match c {
        '1' | 'e' => Some(LogLevel::Error),
        '2' | 'w' => Some(LogLevel::Warn),
        '3' | 'i' => Some(LogLevel::Info),
        '4' | 'd' => Some(LogLevel::Debug),
        '5' | 't' => Some(LogLevel::Trace),
        _ => None,
    }
}
