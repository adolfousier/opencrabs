//! Usage dashboard layout and state
//!
//! Centered overlay panel (~75% of screen), responsive to different sizes.

use super::cards;
use super::data::{DashboardData, Period};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Clear},
};

/// Number of focusable cards
const CARD_COUNT: usize = 5;

/// Dashboard UI state (lives in App)
#[derive(Debug, Clone)]
pub struct DashboardState {
    pub period: Period,
    pub focused_card: usize,
    pub data: DashboardData,
}

impl Default for DashboardState {
    fn default() -> Self {
        Self {
            period: Period::AllTime,
            focused_card: 0,
            data: DashboardData::default(),
        }
    }
}

impl DashboardState {
    pub fn focus_next(&mut self) {
        self.focused_card = (self.focused_card + 1) % CARD_COUNT;
    }

    pub fn focus_prev(&mut self) {
        self.focused_card = if self.focused_card == 0 {
            CARD_COUNT - 1
        } else {
            self.focused_card - 1
        };
    }

    /// Set period and return true if changed (caller should re-fetch data)
    pub fn set_period(&mut self, period: Period) -> bool {
        if self.period != period {
            self.period = period;
            true
        } else {
            false
        }
    }
}

/// Compute a centered rect that takes ~75% of the terminal, clamped to min/max.
fn centered_rect(area: Rect) -> Rect {
    // Target ~75% but at least 60 cols / 20 rows, at most area - 4 margin each side
    let w = (area.width * 3 / 4)
        .max(60.min(area.width))
        .min(area.width.saturating_sub(4));
    let h = (area.height * 3 / 4)
        .max(20.min(area.height))
        .min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

/// Render the usage dashboard as a centered overlay.
///
/// Layout inside the panel:
/// ```text
/// [Summary Bar ─────────────────── full width]
/// [Daily Activity ]  [By Project            ]
/// [By Model       ]  [Core Tools            ]
/// [By Activity ─────────────────── full width]
/// [Footer ──────────────────────── full width]
/// ```
pub fn render(f: &mut Frame, state: &DashboardState, area: Rect) {
    let panel = centered_rect(area);

    // Clear the area behind the panel
    f.render_widget(Clear, panel);

    // Outer border
    let border = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Usage Dashboard ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    let border_inner = border.inner(panel);
    f.render_widget(border, panel);

    // 1-cell horizontal padding inside the border
    let inner = Rect {
        x: border_inner.x + 1,
        y: border_inner.y,
        width: border_inner.width.saturating_sub(2),
        height: border_inner.height,
    };

    // Adaptive: if panel is too short, shrink activity card
    let activity_height = if inner.height > 30 { 6 } else { 4 };
    let grid_min = if inner.height > 30 { 8 } else { 5 };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),                      // summary bar
            Constraint::Min(grid_min),                  // middle 2x2 grid
            Constraint::Length(activity_height as u16), // activity (full width)
            Constraint::Length(1),                      // footer
        ])
        .split(inner);

    // Summary bar
    cards::render_summary(f, &state.data, chunks[0], state.period.label());

    // Middle grid: 2 rows x 2 columns
    let mid_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);

    let top_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(mid_rows[0]);

    let bot_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(mid_rows[1]);

    // Card index: 0=daily, 1=project, 2=model, 3=tools, 4=activity
    cards::render_daily(f, &state.data.daily, top_cols[0], state.focused_card == 0);
    cards::render_projects(
        f,
        &state.data.projects,
        top_cols[1],
        state.focused_card == 1,
    );
    cards::render_models(f, &state.data.models, bot_cols[0], state.focused_card == 2);
    cards::render_tools(f, &state.data.tools, bot_cols[1], state.focused_card == 3);

    // Activity (full width)
    cards::render_activities(
        f,
        &state.data.activities,
        chunks[2],
        state.focused_card == 4,
    );

    // Footer
    cards::render_footer(f, chunks[3]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_state() {
        let s = DashboardState::default();
        assert_eq!(s.period, Period::AllTime);
        assert_eq!(s.focused_card, 0);
    }

    #[test]
    fn test_focus_cycling() {
        let mut s = DashboardState::default();
        assert_eq!(s.focused_card, 0);
        s.focus_next();
        assert_eq!(s.focused_card, 1);
        s.focus_next();
        assert_eq!(s.focused_card, 2);
        s.focus_next();
        assert_eq!(s.focused_card, 3);
        s.focus_next();
        assert_eq!(s.focused_card, 4);
        s.focus_next();
        assert_eq!(s.focused_card, 0); // wraps

        s.focus_prev();
        assert_eq!(s.focused_card, 4); // wraps back
        s.focus_prev();
        assert_eq!(s.focused_card, 3);
    }

    #[test]
    fn test_set_period() {
        let mut s = DashboardState::default();
        assert!(s.set_period(Period::Today));
        assert!(!s.set_period(Period::Today)); // same, no change
        assert!(s.set_period(Period::Week));
    }

    #[test]
    fn test_centered_rect_large_terminal() {
        let area = Rect::new(0, 0, 200, 60);
        let r = centered_rect(area);
        assert_eq!(r.width, 150); // 75% of 200
        assert_eq!(r.height, 45); // 75% of 60
        assert_eq!(r.x, 25); // centered
        assert_eq!(r.y, 7); // centered (rounding)
    }

    #[test]
    fn test_centered_rect_small_terminal() {
        let area = Rect::new(0, 0, 80, 24);
        let r = centered_rect(area);
        assert_eq!(r.width, 60); // 75% of 80 = 60, meets floor of 60
        assert_eq!(r.height, 20); // max(75%=18, floor=20) = 20, capped at 22
    }

    #[test]
    fn test_centered_rect_tiny_terminal() {
        let area = Rect::new(0, 0, 40, 15);
        let r = centered_rect(area);
        // Should not exceed area
        assert!(r.width <= area.width);
        assert!(r.height <= area.height);
        assert!(r.x >= area.x);
        assert!(r.y >= area.y);
    }
}
