//! The controls a window draws itself when the platform draws none (#2813,
//! #2966).
//!
//! Two platforms arrive here from opposite directions. On Wayland with a
//! compositor that does not implement `zxdg_decoration_manager_v1` the window
//! opened with no title bar and no minimise, maximise or close button (#2813).
//! On Windows gpui creates every window without `WS_CAPTION`, so the OS draws
//! no title bar and the app is asked for one that does not exist (#2966). The
//! decision of which controls to draw is executed here; the drawing that needs
//! a window lives in `src/gui/window_controls.rs`.

#![cfg(feature = "gui")]

use crate::ui_core::window_controls::{Control, controls, effective_decorations, glyph};
use gpui::{Decorations, Tiling, WindowControlArea};

fn client() -> Decorations {
    Decorations::Client {
        tiling: Tiling::default(),
    }
}

/// The bug this exists for: a client-decorated window has to draw all three,
/// or the window cannot be closed from inside it (#2813).
#[test]
fn a_client_decorated_window_draws_all_three() {
    assert_eq!(
        controls(client()),
        &[Control::Minimize, Control::Maximize, Control::Close]
    );
}

/// A server-decorated window already has the compositor's buttons, so a second
/// set would be a duplicate in the same corner.
#[test]
fn a_server_decorated_window_draws_none() {
    assert_eq!(controls(Decorations::Server), &[]);
}

/// A tiled window has no restore size, so a maximize there would do nothing.
#[test]
fn a_tiled_window_drops_maximize() {
    for tiling in [
        Tiling {
            top: true,
            ..Tiling::default()
        },
        Tiling {
            bottom: true,
            ..Tiling::default()
        },
        Tiling {
            left: true,
            ..Tiling::default()
        },
        Tiling {
            right: true,
            ..Tiling::default()
        },
    ] {
        assert_eq!(
            controls(Decorations::Client { tiling }),
            &[Control::Minimize, Control::Close],
            "a tiled edge drops maximize: {tiling:?}"
        );
    }
}

/// Close is last in every configuration that draws it: the destructive control
/// never sits where the pointer arrives first.
#[test]
fn close_is_always_last() {
    assert_eq!(controls(client()).last(), Some(&Control::Close));
    let tiled = Decorations::Client {
        tiling: Tiling {
            left: true,
            ..Tiling::default()
        },
    };
    assert_eq!(controls(tiled).last(), Some(&Control::Close));
}

/// A platform that draws no caption of its own answers `Server` for a title bar
/// that does not exist, which is what Windows does. Read as the client
/// decoration it really is, the window draws all three controls and can be
/// closed from inside it (#2966).
#[test]
fn a_captionless_platform_draws_all_three_from_a_server_report() {
    let decorations = effective_decorations(Decorations::Server, false);
    assert_eq!(decorations, client());
    assert_eq!(
        controls(decorations),
        &[Control::Minimize, Control::Maximize, Control::Close]
    );
}

/// Where the platform does draw a caption, a `Server` report is the truth. A
/// second set of buttons would duplicate the ones already in that corner.
#[test]
fn a_captioned_platform_keeps_a_server_report_and_draws_none() {
    let decorations = effective_decorations(Decorations::Server, true);
    assert_eq!(decorations, Decorations::Server);
    assert_eq!(controls(decorations), &[]);
}

/// A client report is already the answer, tiling included: the caption
/// question must not flatten a tiled window into an untiled one.
#[test]
fn a_client_report_passes_through_with_its_tiling() {
    let tiled = Decorations::Client {
        tiling: Tiling {
            left: true,
            ..Tiling::default()
        },
    };
    for platform_draws_caption in [true, false] {
        assert_eq!(
            effective_decorations(tiled, platform_draws_caption),
            tiled,
            "a client report is passed through: {platform_draws_caption}"
        );
    }
    assert_eq!(
        controls(effective_decorations(tiled, false)),
        &[Control::Minimize, Control::Close]
    );
}

/// Every control carries its own window-control area: the area is what the
/// platform reads to tell a control from content, so two controls sharing one
/// would make one button act as the other.
#[test]
fn every_control_carries_its_own_area() {
    let areas = [
        glyph(Control::Minimize).1,
        glyph(Control::Maximize).1,
        glyph(Control::Close).1,
    ];
    assert_eq!(areas[0], WindowControlArea::Min);
    assert_eq!(areas[1], WindowControlArea::Max);
    assert_eq!(areas[2], WindowControlArea::Close);
    for (index, area) in areas.iter().enumerate() {
        for (other_index, other) in areas.iter().enumerate() {
            if index != other_index {
                assert_ne!(area, other, "two controls share one area");
            }
        }
    }
}
