//! Snippets that get data out of a page, including across pages.
//!
//! The counterpart of [`crate::ops::harvest`]. Pagination lives here rather than with the
//! navigation snippets because it is only ever used in the same loop: extract this page,
//! advance, extract the next.

use super::Snippet;

/// Every link on the page, capped (`extract what=links`).
pub fn extract_links() -> Snippet {
    Snippet::new(include_str!("../../js/extract_links.js"))
}

/// One HTML table as an array of header→cell objects (`extract_table`).
pub fn extract_table() -> Snippet {
    Snippet::new(include_str!("../../js/extract_table.js"))
}

/// Click a caller-supplied next-page control (`paginate` with a selector).
pub fn paginate_click() -> Snippet {
    Snippet::new(include_str!("../../js/paginate_click.js"))
}

/// Find and click the next-page control (`paginate` without a selector).
pub fn paginate_next() -> Snippet {
    Snippet::new(include_str!("../../js/paginate_next.js"))
}
