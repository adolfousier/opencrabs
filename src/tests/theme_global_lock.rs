//! Serialises the process-global theme state across test modules.
//!
//! `theme::set`/`theme::reset` mutate a process-wide slot and bump a
//! process-wide `GENERATION` counter. The parallel test harness runs every
//! module in one process, so a module that only *observes* that counter
//! (`tui_theme_cache_invalidation_test`, which asserts an `App`'s render-cache
//! stamp still matches the live value) races the one module that mutates it
//! (`tui_theme_presets_test::set_and_reset_switch_active_theme`, which bumps it
//! eleven times). Both are correct on their own; the assertion window is what
//! overlaps. Observed as `left: 0, right: 11` in a full-suite run while every
//! test passed in isolation.
//!
//! Every test that reads or writes the active theme or its generation takes
//! this lock first. The guard is an RAII binding, so it must be held in a
//! `let` for the duration of the critical section:
//!
//! ```ignore
//! let _guard = theme_global_lock::lock();
//! ```
//!
//! Hold it across synchronous work only. Acquire it *after* any `.await` in a
//! test so the guard never straddles a suspension point.

use std::sync::{Mutex, MutexGuard};

static THEME_GLOBALS: Mutex<()> = Mutex::new(());

/// Exclusive access to the theme slot and its generation counter.
///
/// Poisoning is recovered rather than propagated: a test that panics while
/// holding this lock has already failed on its own merits, and re-panicking
/// every later theme test would bury the real failure under a cascade.
pub(crate) fn lock() -> MutexGuard<'static, ()> {
    THEME_GLOBALS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
