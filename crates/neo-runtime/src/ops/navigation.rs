//! Navigation request capture op.

use crate::ops::NavigationQueue;
use deno_core::op2;
use deno_core::OpState;
use std::cell::RefCell;
use std::rc::Rc;

/// Capture a navigation request from JS (form.submit, location.href, etc.)
///
/// Stores the request JSON in the navigation queue for the engine to
/// pick up after script execution or interaction completes.
#[op2]
#[string]
pub fn op_navigation_request(state: Rc<RefCell<OpState>>, #[string] request_json: String) -> String {
    let s = state.borrow();
    if let Some(nav_queue) = s.try_borrow::<NavigationQueue>() {
        nav_queue.push(request_json);
    }
    "ok".to_string()
}
