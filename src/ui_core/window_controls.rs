//! Which window controls a window has to draw itself, and what each one maps
//! to (#2813).
//!
//! gpui asks for server-side decorations when `WindowOptions::window_decorations`
//! is `None` (`gpui-0.2.2/src/window.rs:997`). On Wayland that request travels
//! through `zxdg_decoration_manager_v1`, and a compositor that does not
//! implement it never answers: mutter 16 is one, the protocol name is absent
//! from `libmutter-16.so`, gpui's decoration global stays `None`, and the window
//! opens with no chrome at all. No title bar, no minimise, maximise or close
//! button, and no way out but a keyboard shortcut or killing the process.
//!
//! The window therefore asks for client-side decorations on Linux and paints
//! the three controls itself. The drawing needs a window, so it lives in the
//! GUI binary (`src/gui/window_controls.rs`); the decision lives here, where
//! the suite can execute it.
//!
//! Windows arrives at the same place from the other side (#2966). gpui creates
//! every window with `WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX |
//! WS_MAXIMIZEBOX` and never `WS_CAPTION`
//! (`gpui-0.2.2/src/platform/windows/window.rs:394`), so the OS draws no title
//! bar and no caption button at all, while `window_decorations()` answers the
//! trait's default `Server` (`gpui-0.2.2/src/platform.rs:538`) because the
//! Windows window never overrides it. Such a window is caption-less whichever
//! way it reports, so `effective_decorations` turns that report into the client
//! decoration it really is.

use gpui::{Decorations, Tiling, WindowControlArea};

/// One control a cluster can draw, in the order it paints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Minimize,
    Maximize,
    Close,
}

/// Which controls a window has to draw itself.
///
/// A server-decorated window already carries the compositor's own buttons, so
/// the cluster draws nothing and the corner stays free. A client-decorated one
/// draws all three, minus the maximize button while an edge is tiled: a tiled
/// window has no restore size, so a maximize there would be a dead control.
pub fn controls(decorations: Decorations) -> &'static [Control] {
    match decorations {
        Decorations::Server => &[],
        Decorations::Client { tiling } => {
            if tiling.top || tiling.bottom || tiling.left || tiling.right {
                &[Control::Minimize, Control::Close]
            } else {
                &[Control::Minimize, Control::Maximize, Control::Close]
            }
        }
    }
}

/// Whether the platform draws a caption of its own for a window that reports
/// server-side decorations.
///
/// macOS and the X11 and Wayland servers do. Windows does not, whatever the
/// window reports: see the module doc (#2966).
pub const PLATFORM_DRAWS_CAPTION: bool = !cfg!(target_os = "windows");

/// The decorations a window actually has, once the platform's own caption is
/// accounted for.
///
/// `reported` is what the platform answers. Where the platform draws no caption
/// at all, a `Server` report describes a title bar that does not exist, so it
/// becomes a client decoration and the app draws the controls itself. Every
/// other report is passed through untouched: a client report is already the
/// answer, tiling included.
pub fn effective_decorations(reported: Decorations, platform_draws_caption: bool) -> Decorations {
    match reported {
        Decorations::Server if !platform_draws_caption => Decorations::Client {
            tiling: Tiling::default(),
        },
        other => other,
    }
}

/// The glyph a control paints and the hitbox the platform reads for it.
///
/// The area is what tells the window manager which corner of the surface is a
/// control rather than content: on X11 and Windows it answers the hit test, and
/// on Wayland it is what the window reports back to the app.
pub fn glyph(control: Control) -> (&'static str, WindowControlArea) {
    match control {
        Control::Minimize => ("\u{2013}", WindowControlArea::Min),
        Control::Maximize => ("\u{25a1}", WindowControlArea::Max),
        Control::Close => ("\u{2715}", WindowControlArea::Close),
    }
}
