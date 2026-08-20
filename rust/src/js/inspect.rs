//! Snippets that answer "why did that behave like that": timings, resolved CSS, source
//! maps, console output.
//!
//! These serve [`crate::devtools`] and the `debug` tool. They are grouped by purpose rather
//! than by caller — the console interceptor is reached through `ops::introspect` while the
//! rest come from `devtools` — because what a reader looks for here is the diagnostic, not
//! the dispatch path.

use super::Snippet;

/// Web Vitals and navigation timing.
pub fn vitals() -> Snippet {
    Snippet::new(include_str!("../../js/vitals.js"))
}

/// Resolved CSS for one element, plus why it is invisible when it is.
pub fn computed_style() -> Snippet {
    Snippet::new(include_str!("../../js/computed_style.js"))
}

/// Fetch a script's source map from inside the page, so it inherits the page's credentials.
pub fn fetch_source_map() -> Snippet {
    Snippet::new(include_str!("../../js/fetch_source_map.js"))
}

/// Install the console interceptor (`debug action=start`).
pub fn debug_capture_on() -> Snippet {
    Snippet::new(include_str!("../../js/debug_capture_on.js"))
}

/// Restore the page's own console (`debug action=stop`).
pub fn debug_capture_off() -> Snippet {
    Snippet::new(include_str!("../../js/debug_capture_off.js"))
}
