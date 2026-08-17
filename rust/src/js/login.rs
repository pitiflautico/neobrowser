//! Snippets that drive a login form and judge whether it worked.
//!
//! The counterpart of [`mod@crate::sessions::login`]. Separate from [`super::forms`] because a
//! login is where getting the fill wrong is most expensive: the field looks filled, the
//! submit sends empty credentials, and the result is indistinguishable from a rejected
//! password. [`login_state`] exists for the same reason — "still on the form" and "cannot
//! tell" are different answers, and collapsing them is what makes an agent proceed as
//! though it were authenticated.

use super::Snippet;

/// Find and fill a login form's username/email field.
pub fn login_find_field() -> Snippet {
    Snippet::new(include_str!("../../js/login_find_field.js"))
}

/// Find and fill a login form's password field.
pub fn login_fill_field() -> Snippet {
    Snippet::new(include_str!("../../js/login_fill_field.js"))
}

/// Submit the form that owns the password field, not the first one in the document.
pub fn login_submit() -> Snippet {
    Snippet::new(include_str!("../../js/login_submit.js"))
}

/// Whether a *visible* password field is still present — the honest login signal.
pub fn login_state() -> Snippet {
    Snippet::new(include_str!("../../js/login_state.js"))
}
