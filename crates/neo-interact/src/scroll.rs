//! Scroll simulation.
//!
//! Dispatches scroll events and counts interactive elements as a proxy
//! for "new content loaded". Includes infinite-scroll detection.

use neo_dom::DomEngine;

use crate::{InteractError, ScrollDirection};

/// Scroll the page in the given direction.
///
/// Dispatches a scroll event on the body element and returns the total
/// number of visible interactive elements as a proxy for content changes.
pub fn scroll(
    dom: &mut dyn DomEngine,
    direction: ScrollDirection,
    _amount: u32,
) -> Result<usize, InteractError> {
    let _ = direction;

    // Dispatch scroll event on body (sets data-scroll-y as a side effect
    // so extractors can observe it).
    if let Some(body) = dom.query_selector("body") {
        // We simulate the scroll by bumping a synthetic attribute.
        // Real JS-driven pages would fire `scroll` on `window`; here
        // we record it on body so tests can verify the event fired.
        let current: i64 = dom
            .get_attribute(body, "data-scroll-y")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let delta: i64 = match direction {
            ScrollDirection::Down => 500,
            ScrollDirection::Up => -500,
        };
        let next = (current + delta).max(0);
        dom.set_attribute(body, "data-scroll-y", &next.to_string());
    }

    // Count interactive elements as proxy for "what's visible"
    Ok(dom.get_buttons().len() + dom.get_links().len() + dom.get_inputs().len())
}

/// Scroll until no new content loads, or `max_scrolls` reached.
///
/// Compares the interactive-element count after each scroll. If it
/// doesn't increase, we assume infinite-scroll content is exhausted.
/// Returns the last element count.
pub fn scroll_until_stable(
    dom: &mut dyn DomEngine,
    max_scrolls: u32,
) -> Result<usize, InteractError> {
    let mut last_count = 0;
    for _ in 0..max_scrolls {
        let count = scroll(dom, ScrollDirection::Down, 1)?;
        if count == last_count {
            break; // No new content
        }
        last_count = count;
    }
    Ok(last_count)
}
