//! Cookie ops — document.cookie access from JS.

use crate::ops::{CookieState, SharedCookieStore};
use deno_core::op2;
use deno_core::OpState;
use std::cell::RefCell;
use std::rc::Rc;

/// Get cookies for current origin (called by document.cookie getter).
#[op2]
#[string]
pub fn op_cookie_get(state: Rc<RefCell<OpState>>) -> String {
    let s = state.borrow();
    if let Some(cookies) = s.try_borrow::<CookieState>() {
        cookies.get_cookie_string()
    } else {
        String::new()
    }
}

/// Set a cookie (called by document.cookie setter).
#[op2(fast)]
pub fn op_cookie_set(state: Rc<RefCell<OpState>>, #[string] cookie_str: String) {
    let s = state.borrow();
    if let Some(cookies) = s.try_borrow::<CookieState>() {
        cookies.set_from_string(&cookie_str);
    }
}

/// Get cookies for a given URL from the shared cookie store.
///
/// Called from JS as a fallback when `__neorender_cookies` is empty.
/// Returns "name=val; name2=val2" format or empty string.
#[op2]
#[string]
pub fn op_cookie_get_for_url(state: Rc<RefCell<OpState>>, #[string] url: String) -> String {
    let s = state.borrow();
    if let Some(store) = s.try_borrow::<SharedCookieStore>() {
        if let Some(ref cs) = store.0 {
            return cs.get_for_request(&url, None, true);
        }
    }
    String::new()
}
