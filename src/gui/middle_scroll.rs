//! Telegram-style middle-click auto-scroll for one transcript (#2964).
//!
//! The window owns the gesture because the marker, pointer tracking and frame
//! callback outlive any one row. The target names the exact chat or split leaf
//! whose scroll handle may move.

use super::*;
use opencrabs::ui_core::scroll::{AutoScrollLanding, auto_scroll_delta, auto_scroll_landing};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MiddleScrollTarget {
    Chat(Uuid),
    Pane(Uuid),
}

pub(super) struct MiddleScroll {
    target: MiddleScrollTarget,
    anchor: Point<Pixels>,
    pointer: Point<Pixels>,
    last_tick: std::time::Instant,
}

impl MiddleScroll {
    fn new(target: MiddleScrollTarget, anchor: Point<Pixels>) -> Self {
        Self {
            target,
            anchor,
            pointer: anchor,
            last_tick: std::time::Instant::now(),
        }
    }

    fn delta(&self, elapsed_secs: f32) -> f32 {
        auto_scroll_delta(
            f32::from(self.anchor.y),
            f32::from(self.pointer.y),
            elapsed_secs,
        )
    }

    pub(super) fn moving(&self) -> bool {
        self.delta(1.0) != 0.0
    }
}

impl Dashboard {
    /// Middle-click toggles the gesture. A new press while inactive anchors it
    /// to the transcript that received the event, never to the focused pane.
    pub(crate) fn middle_scroll_down(
        &mut self,
        target: MiddleScrollTarget,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.middle_scroll.take().is_none() {
            self.middle_scroll = Some(MiddleScroll::new(target, position));
        }
        cx.notify();
    }

    pub(crate) fn cancel_middle_scroll(&mut self, cx: &mut Context<Self>) {
        if self.middle_scroll.take().is_some() {
            cx.notify();
        }
    }

    /// The ordinary follow pass must stand down only for the active target.
    /// Its 2 px bottom slack would otherwise re-arm after the first tiny step
    /// upward and snap the reader straight back to the newest row.
    pub(super) fn middle_scroll_controls_chat(&self) -> bool {
        self.middle_scroll.as_ref().is_some_and(|gesture| {
            matches!(
                gesture.target,
                MiddleScrollTarget::Chat(session) if self.chat_session == Some(session)
            )
        })
    }

    pub(super) fn middle_scroll_controls_pane(&self, session: Uuid) -> bool {
        self.middle_scroll.as_ref().is_some_and(|gesture| {
            gesture.target == MiddleScrollTarget::Pane(session)
        })
    }

    /// Restart the clock only when a stationary gesture starts moving. A
    /// pointer that is already outside the dead zone keeps the last frame's
    /// timestamp, so frequent pointer events cannot discard elapsed time.
    pub(crate) fn move_middle_scroll(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if let Some(gesture) = self.middle_scroll.as_mut()
            && gesture.pointer != position
        {
            let was_moving = gesture.moving();
            gesture.pointer = position;
            if !was_moving {
                gesture.last_tick = std::time::Instant::now();
            }
            cx.notify();
        }
    }

    /// Queue one redraw for the next animation frame. This mirrors
    /// `Window::request_animation_frame`, with the receipt cleared by the
    /// callback itself: an unrelated repaint must not queue a second loop.
    pub(crate) fn schedule_middle_scroll_frame(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let animate = self
            .middle_scroll
            .as_ref()
            .is_some_and(|gesture| gesture.moving());
        if !animate || self.middle_scroll_frame_pending {
            return;
        }
        self.middle_scroll_frame_pending = true;
        let dashboard = cx.weak_entity();
        window.on_next_frame(move |_, cx| {
            if let Some(dashboard) = dashboard.upgrade() {
                dashboard.update(cx, |this, cx| {
                    this.middle_scroll_frame_pending = false;
                    cx.notify();
                });
            }
        });
    }

    /// Advance the active target once. The scheduled callback notifies the
    /// dashboard, and the next render applies the elapsed pointer movement.
    pub(super) fn tick_middle_scroll(&mut self, cx: &mut Context<Self>) {
        let Some(gesture) = self.middle_scroll.as_mut() else {
            return;
        };
        let now = std::time::Instant::now();
        let elapsed = now.duration_since(gesture.last_tick).as_secs_f32();
        gesture.last_tick = now;
        let delta = gesture.delta(elapsed);
        let target = gesture.target;
        let target_visible = match target {
            MiddleScrollTarget::Chat(session) => {
                self.screen == Screen::Chat && self.chat_session == Some(session)
            }
            MiddleScrollTarget::Pane(session) => {
                self.screen == Screen::Chat && self.pane_states.contains_key(&session)
            }
        };
        // Even a dead-zone marker must not survive a hidden screen or a leaf
        // that was closed. Checking before the zero-delta exit keeps that true.
        if !target_visible {
            self.cancel_middle_scroll(cx);
            return;
        }
        if delta == 0.0 {
            return;
        }

        let finished = match target {
            MiddleScrollTarget::Chat(_) => {
                // This dashboard render already paints the new offset.
                // Notifying it here would queue a second render beside
                // the animation callback on every moving frame.
                let landing = apply_delta(
                    &self.chat_state.scroll,
                    &mut self.chat_state.follow,
                    delta,
                );
                !landing.moved || (delta < 0.0 && landing.at_end)
            }
            MiddleScrollTarget::Pane(session) => {
                match self.pane_states.get(&session).cloned() {
                    Some(pane) => {
                        let landing = pane.update(cx, |pane, cx| {
                            let landing = apply_delta(&pane.scroll, &mut pane.follow, delta);
                            if landing.moved {
                                cx.notify();
                            }
                            landing
                        });
                        !landing.moved || (delta < 0.0 && landing.at_end)
                    }
                    None => true,
                }
            }
        };
        if finished {
            self.cancel_middle_scroll(cx);
        }
    }
}

/// Move one viewport and fold the landing into its existing follow rule.
fn apply_delta(scroll: &ScrollHandle, follow: &mut Follow, delta: f32) -> AutoScrollLanding {
    let current = scroll.offset();
    let max = f32::from(scroll.max_offset().height);
    let landing = auto_scroll_landing(f32::from(current.y), max, delta);
    if landing.moved {
        follow.release();
        scroll.set_offset(point(current.x, px(landing.offset)));
    }
    if delta < 0.0 && landing.at_end {
        follow.rearm();
    }
    landing
}

fn marker_arrow(glyph: &'static str, active: bool) -> Div {
    let palette = palette::current();
    div()
        .text_size(px(11.0))
        .text_color(rgb(if active {
            palette.accent
        } else {
            palette.text_muted
        }))
        .child(glyph)
}

/// A full-window hitbox hides the platform cursor while the gesture owns the
/// pointer. Its only visible child is the fixed circular marker at the anchor.
/// Root capture still receives presses, and root bubble still receives moves
/// and wheel events, so every documented exit remains available.
pub(super) fn overlay(gesture: &MiddleScroll) -> Stateful<Div> {
    const MARKER_RADIUS: f32 = 17.0;
    let palette = palette::current();
    let delta = gesture.delta(1.0);
    div()
        .id("middle-scroll-overlay")
        .absolute()
        .inset_0()
        .occlude()
        .cursor(gpui::CursorStyle::None)
        .child(
            div()
                .absolute()
                .left(px(f32::from(gesture.anchor.x) - MARKER_RADIUS))
                .top(px(f32::from(gesture.anchor.y) - MARKER_RADIUS))
                .size(px(MARKER_RADIUS * 2.0))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_1()
                .border_color(rgb(palette.panel_hi))
                .bg(rgb(palette.panel))
                .child(marker_arrow("↑", delta > 0.0))
                .child(
                    div()
                        .size(px(3.0))
                        .rounded_full()
                        .bg(rgb(palette.text_muted)),
                )
                .child(marker_arrow("↓", delta < 0.0)),
        )
}
