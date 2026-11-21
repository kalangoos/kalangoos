//! The agents center's width rules (design "Central · conversa responsiva", "Central · canvas largo" and "Central ·
//! canvas expandido"): proportions of the room there is, with only floors in pixels. Pure functions, so the rules
//! are tested without a window; the conversation page recomputes them on every resize.

/// The reading column's share of the room between the side panels.
pub const COLUMN_SHARE: f64 = 0.78;
/// The column's floor while the room allows it.
pub const COLUMN_MIN: i32 = 640;
/// The least margin left on each side of the column.
pub const COLUMN_MARGIN: i32 = 32;
/// The measure of a paragraph, in characters of its own font (follows the font size and scale, not pixels).
pub const PROSE_CHARS: i32 = 100;

/// What the conversation keeps at least beside the side panels (they give way first).
pub const CONVERSATION_MIN: i32 = 480;
/// The conversation's width beside an expanded canvas.
pub const CONVERSATION_EXPANDED: i32 = 420;

/// The reading column for `room` px between the side panels: 78 % of it, at least 640 px, but always leaving the
/// margins (a narrow room gives the column all but them). No ceiling: blocks inside take the whole column, prose
/// keeps its own measure in characters.
pub fn reading_column(room: i32) -> i32 {
    let share = (f64::from(room) * COLUMN_SHARE).round() as i32;
    share.max(COLUMN_MIN).min(room - 2 * COLUMN_MARGIN).max(0)
}

/// What the right panel shows, for its width rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelKind {
    /// Context, Files, Changes, Terminal: a column of rows
    Narrow,
    /// Canvas and Browser: a page needs room
    Wide,
}

/// A panel's default width as a share of the window, clamped to (min, max) — and the limits a drag may take it to.
pub struct PanelRule {
    pub share: f64,
    pub default: (i32, i32),
    pub limits: (i32, i32),
}

/// The right panel's rule in a `window` px wide window.
pub fn panel_rule(kind: PanelKind, window: i32) -> PanelRule {
    match kind {
        PanelKind::Narrow => PanelRule { share: 0.26, default: (300, 480), limits: (280, 560) },
        PanelKind::Wide => {
            let most = (f64::from(window) * 0.70).round() as i32;
            PanelRule { share: 0.45, default: (480, most.max(480)), limits: (360, most.max(360)) }
        }
    }
}

/// The right panel's width: the person's (`saved`) or the rule's share, within the rule's limits and the `room`
/// the window leaves it once the conversation has its least (never under the least limit). `left`: what the left
/// side takes (the side bar and its edge, or the rail).
pub fn right_panel_width(kind: PanelKind, window: i32, left: i32, saved: Option<i32>, edge: i32) -> i32 {
    let r = panel_rule(kind, window);
    crate::resize_handle::panel_width(window, saved, r.share, r.default, r.limits, window - left - edge - CONVERSATION_MIN)
}

/// The expanded canvas: the window less the rail, the edge and the conversation's narrow column.
pub fn expanded_panel_width(window: i32, rail: i32, edge: i32) -> i32 {
    (window - rail - edge - CONVERSATION_EXPANDED).max(panel_rule(PanelKind::Wide, window).limits.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_column_is_a_share_of_the_room_with_a_floor_and_no_ceiling() {
        // 1920 with the side bar (360) and the context (480): 1080 px of room
        assert_eq!(reading_column(1080), 842);
        // 3380, same panels: it keeps growing (the old rule stopped at 860)
        assert_eq!(reading_column(2540), 1981);
        // a medium window: the floor
        assert_eq!(reading_column(800), 640);
        // a room under the floor and its margins: all but the margins
        assert_eq!(reading_column(600), 536);
        assert_eq!(reading_column(40), 0);
    }

    #[test]
    fn the_column_follows_the_room_without_jumps() {
        // every pixel of room moves the column by at most one pixel (smooth while dragging an edge)
        let mut last = reading_column(0);
        for room in 1..4000 {
            let c = reading_column(room);
            assert!((c - last).abs() <= 1, "room {room}: {last} → {c}");
            last = c;
        }
    }

    #[test]
    fn canvas_and_browser_take_45_percent_up_to_70() {
        // 1920 with the rail (64 + edge 9): 45 % is 864
        assert_eq!(right_panel_width(PanelKind::Wide, 1920, 73, None, 9), 864);
        // 3380: 1521
        assert_eq!(right_panel_width(PanelKind::Wide, 3380, 73, None, 9), 1521);
        // dragged wide: held at 70 % (2366) — the conversation's 480 still fits beside it
        assert_eq!(right_panel_width(PanelKind::Wide, 3380, 73, Some(3000), 9), 2366);
        // dragged narrow: held at 360
        assert_eq!(right_panel_width(PanelKind::Wide, 3380, 73, Some(200), 9), 360);
        // 1280: 576, and the conversation keeps 480 beside the side bar
        assert_eq!(right_panel_width(PanelKind::Wide, 1280, 73, None, 9), 576);
        assert_eq!(right_panel_width(PanelKind::Wide, 1280, 300, None, 9), 491);
    }

    #[test]
    fn the_context_keeps_its_narrow_rule() {
        assert_eq!(right_panel_width(PanelKind::Narrow, 1920, 369, None, 9), 480);
        assert_eq!(right_panel_width(PanelKind::Narrow, 1500, 369, None, 9), 390);
        assert_eq!(right_panel_width(PanelKind::Narrow, 3380, 369, Some(900), 9), 560);
    }

    #[test]
    fn each_tab_keeps_its_own_width() {
        // the same saved width means different things per kind: a canvas of 900 is not a context of 900
        assert_eq!(right_panel_width(PanelKind::Wide, 1920, 73, Some(900), 9), 900);
        assert_eq!(right_panel_width(PanelKind::Narrow, 1920, 73, Some(900), 9), 560);
    }

    #[test]
    fn the_expanded_canvas_leaves_the_conversation_its_narrow_column() {
        assert_eq!(expanded_panel_width(1920, 64, 9), 1427);
        assert_eq!(expanded_panel_width(3380, 64, 9), 2887);
        // a small window still gives the canvas its least
        assert_eq!(expanded_panel_width(700, 64, 9), 360);
    }
}
