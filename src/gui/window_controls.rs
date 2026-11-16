//! The window controls the app draws itself when the platform draws none
//! (#2813, #2966).
//!
//! This half is the drawing: it needs a `Window` and the view's own `cx`, so it
//! lives in the GUI binary. The decision of which controls a window has to draw
//! lives in `opencrabs::ui_core::window_controls`, where the test suite can
//! execute it.
//!
//! The controls are an overlay and not a title row on purpose: the owner
//! removed the chat screen's title row in #2596 and does not want it back.

use std::rc::Rc;

use gpui::{App, ClickEvent, Decorations, Div, Stateful, Window, div, prelude::*, rgb};
/// The window's own `match` names the controls, so the type is re-exported
/// rather than reached through the library path at every use.
pub use opencrabs::ui_core::window_controls::Control;
use opencrabs::ui_core::window_controls::{controls, glyph};

use crate::palette;

/// One control button, styled but without its click handler: only the view's
/// own `cx` can mint one, so the caller attaches it.
pub fn button(control: Control) -> Stateful<Div> {
    let shown = palette::current();
    let (label, area) = glyph(control);
    // Close is the one control that reads as destructive, so it takes the red
    // wash on hover instead of the panel's neutral one.
    let hover = match control {
        Control::Close => shown.red,
        Control::Minimize | Control::Maximize => shown.panel_hi,
    };
    div()
        .id(match control {
            Control::Minimize => "window-control-min",
            Control::Maximize => "window-control-max",
            Control::Close => "window-control-close",
        })
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(gpui::px(22.0))
        .h(gpui::px(22.0))
        .rounded_sm()
        .text_sm()
        .text_color(rgb(shown.text_muted))
        .cursor_pointer()
        .window_control_area(area)
        .hover(move |d| d.bg(rgb(hover)).text_color(rgb(shown.text)))
        .child(label)
}

/// A control's click, type-erased so a window whose platform runs the buttons
/// itself can pass `None` without naming a closure type.
pub type Handler = Rc<dyn Fn(Control, &ClickEvent, &mut Window, &mut App)>;

/// The cluster for a window, or `None` when the platform draws its own.
///
/// Absolute, in the top-right corner, above the header's own padding: the
/// header is a row of the window and the controls belong to the frame, so they
/// do not take a slot in any screen's layout.
///
/// `on_control` is `None` where the platform runs the buttons itself. On
/// Windows a control area answers the non-client hit test and the platform
/// clicks the button from `WM_NCLBUTTONUP`: `HTMINBUTTON` minimises,
/// `HTMAXBUTTON` toggles zoom and `HTCLOSE` posts `WM_CLOSE`
/// (`gpui-0.2.2/src/platform/windows/events.rs:1038`). A listener here would
/// consume the mouse-up before that runs, and gpui's `zoom()` on Windows only
/// ever maximises (`gpui-0.2.2/src/platform/windows/window.rs:782`), so the app
/// could not restore a zoomed window (#2966).
pub fn cluster(decorations: Decorations, on_control: Option<Handler>) -> Option<Div> {
    let list = controls(decorations);
    if list.is_empty() {
        return None;
    }
    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .absolute()
        .top_0()
        .right_0()
        .p(gpui::px(6.0));
    for control in list.iter().copied() {
        let mut element = button(control);
        if let Some(on_control) = on_control.clone() {
            element = element.on_click(move |ev, window, cx| on_control(control, ev, window, cx));
        }
        row = row.child(element);
    }
    Some(row)
}
