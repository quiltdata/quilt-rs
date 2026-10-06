//! Where a floating surface goes, against the element that summoned it.
//!
//! Shared by [`AnchoredOverlay`](super::AnchoredOverlay) and
//! [`Tooltip`](super::Tooltip), which float for the same reason — the top layer,
//! so a scrolling list cannot clip them — and so are placed by the same rule.
//! One copy, because two would drift: a tooltip that flipped above at a
//! different threshold from the menu beside it would look like a bug in one of
//! them, and nobody could say which.
//!
//! Private to the kit. It exists only because CSS anchor positioning has not
//! reached every `WebKit` this app runs on; when it has, this file is deleted
//! and both surfaces say where they go in their stylesheets.

use super::Align;

/// How far from the trigger the surface sits, in pixels.
const OFFSET: f64 = 4.0;
/// How close to the viewport's edge it is allowed to come.
const MARGIN: f64 = 8.0;

/// Put the surface under its trigger, and inside the viewport.
///
/// Below by preference and above when below does not fit, because a menu that
/// opens upward is merely unusual while one that opens off the bottom of the
/// window is unreachable.
pub(super) fn place(element: &web_sys::HtmlElement, anchor: &web_sys::DomRect, align: Align) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let width = f64::from(element.offset_width());
    let height = f64::from(element.offset_height());
    let view_width = window
        .inner_width()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let view_height = window
        .inner_height()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let left = horizontal(align, anchor.left(), anchor.right(), width, view_width);

    let below = anchor.bottom() + OFFSET;
    let above = anchor.top() - OFFSET - height;
    let top = if below + height <= view_height - MARGIN || above < MARGIN {
        below.min((view_height - MARGIN - height).max(MARGIN))
    } else {
        above
    };

    let css = element.style();
    drop(css.set_property("left", &format!("{left}px")));
    drop(css.set_property("top", &format!("{top}px")));
}

/// Where the surface's left edge goes: lined up with one of the trigger's edges,
/// then pulled back inside the viewport.
///
/// Split out from [`place`] because it is the whole decision and the rest is DOM
/// plumbing — it can be checked without a browser, a popover or a layout pass.
///
/// The clamp is a last resort and not an alignment. It ties the surface to the
/// *window* instead of to its trigger, so the gap between the two moves when the
/// window resizes; choosing the edge that has room is what keeps it from firing.
/// `max` comes last, so a surface wider than the viewport still starts on screen
/// rather than off the left of it.
fn horizontal(
    align: Align,
    anchor_left: f64,
    anchor_right: f64,
    width: f64,
    view_width: f64,
) -> f64 {
    let aligned = match align {
        Align::Start => anchor_left,
        Align::End => anchor_right - width,
    };
    aligned.min(view_width - MARGIN - width).max(MARGIN)
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "every input here is integral and the arithmetic is min/max over \
              them, so the results are exact — a tolerance would only hide a \
              placement that had genuinely moved"
)]
mod tests {
    use super::*;

    /// A trailing trigger hangs its surface leftwards from its own right edge.
    /// This is the case the clamp used to swallow: left-aligning a `[⋯]` near the
    /// window's edge pinned the surface to the window, which is why the menu
    /// overhung its own button by 45px at a 1280 viewport.
    #[test]
    fn end_alignment_puts_the_right_edges_together() {
        // A `[⋯]` at 1203..1227 in a 1280 viewport, 200px of menu.
        let left = horizontal(Align::End, 1203.0, 1227.0, 200.0, 1280.0);
        assert_eq!(left + 200.0, 1227.0, "right edges must coincide");
        assert!(left > MARGIN, "and it must not have hit the clamp");
    }

    #[test]
    fn start_alignment_puts_the_left_edges_together() {
        let left = horizontal(Align::Start, 40.0, 72.0, 200.0, 1280.0);
        assert_eq!(left, 40.0);
    }

    /// The clamp still exists for the case it was written for: a surface that
    /// genuinely cannot fit beside its trigger.
    #[test]
    fn a_surface_that_cannot_fit_is_pulled_inside_the_viewport() {
        // Start-aligned against a trigger hard against the right edge.
        let left = horizontal(Align::Start, 1200.0, 1232.0, 200.0, 1280.0);
        assert_eq!(left, 1280.0 - MARGIN - 200.0);

        // End-aligned against a trigger hard against the left edge: the surface
        // would start off-screen, so it is pushed back to the margin.
        let left = horizontal(Align::End, 0.0, 24.0, 200.0, 1280.0);
        assert_eq!(left, MARGIN);
    }

    /// Wider than the viewport: it starts on screen rather than off the left of
    /// it, which is what putting `max` last buys.
    #[test]
    fn a_surface_wider_than_the_viewport_still_starts_on_screen() {
        let left = horizontal(Align::End, 300.0, 340.0, 900.0, 400.0);
        assert_eq!(left, MARGIN);
    }
}
