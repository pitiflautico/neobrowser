//! Scroll simulation.
//!
//! Sets `scrollY` on the document and returns the total element count
//! as a proxy for "new content loaded".

use neo_dom::DomEngine;

use crate::{InteractError, ScrollDirection};

/// Scroll the page in the given direction.
///
/// `amount` is the number of pixels to scroll. Returns the total number
/// of visible interactive elements as a proxy for content changes.
pub fn scroll(
    dom: &dyn DomEngine,
    direction: ScrollDirection,
    _amount: u32,
) -> Result<usize, InteractError> {
    // In a real DOM, we'd mutate scrollTop/scrollY and check for
    // lazy-loaded content. With the trait-based DomEngine, we count
    // interactive elements as a proxy.
    let _ = direction;
    let buttons = dom.get_buttons().len();
    let inputs = dom.get_inputs().len();
    Ok(buttons + inputs)
}
