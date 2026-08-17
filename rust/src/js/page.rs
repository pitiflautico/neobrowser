//! Snippets that ask what the page is, or reach into it.
//!
//! These are the ones no single tool owns: the state digest runs after *every* mutating
//! action, piercing and frame reachability serve anything that has to look inside a shadow
//! root or an iframe, and the wall signals run on every navigation. Grouping them by caller
//! would scatter them across five modules, so they are grouped by what they observe.

use super::Snippet;

/// The page-state digest used by every verified action.
pub fn state_digest() -> Snippet {
    Snippet::new(include_str!("../../js/state_digest.js"))
}

/// Shadow-DOM and same-origin-iframe piercing.
pub fn pierce() -> Snippet {
    Snippet::new(include_str!("../../js/pierce.js"))
}

/// Frame reachability, for `list_frames`.
pub fn frame_access() -> Snippet {
    Snippet::new(include_str!("../../js/frame_access.js"))
}

/// Set a checkbox, radio, select or contenteditable through the framework-visible setter.
///
/// The `set` tool's path. [`super::forms::fill_control`] does the same job for the `fill`
/// tool and is deliberately a separate snippet: `fill` reports the resulting value and `set`
/// does not, and folding them together would mean one of the two tools changing behaviour.
pub fn set_control() -> Snippet {
    Snippet::new(include_str!("../../js/set_control.js"))
}

/// The one round-trip that gathers every bot-wall / captcha / consent / login signal.
pub fn wall_signals() -> Snippet {
    Snippet::new(include_str!("../../js/wall_signals.js"))
}
