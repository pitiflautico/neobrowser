//! Web Storage ops (localStorage).

use crate::ops::StorageState;
use deno_core::op2;
use deno_core::OpState;
use std::cell::RefCell;
use std::rc::Rc;

/// Get a value from localStorage.
#[op2]
#[string]
pub fn op_storage_get(
    state: Rc<RefCell<OpState>>,
    #[string] key: String,
) -> Result<String, deno_error::JsErrorBox> {
    let s = state.borrow();
    let storage = s
        .try_borrow::<StorageState>()
        .ok_or_else(|| deno_error::JsErrorBox::generic("No StorageState"))?;
    let val = storage
        .backend
        .get(&storage.origin, &key)
        .unwrap_or_default();
    Ok(val)
}

/// Set a value in localStorage.
#[op2(fast)]
pub fn op_storage_set(
    state: Rc<RefCell<OpState>>,
    #[string] key: String,
    #[string] value: String,
) -> Result<(), deno_error::JsErrorBox> {
    let s = state.borrow();
    let storage = s
        .try_borrow::<StorageState>()
        .ok_or_else(|| deno_error::JsErrorBox::generic("No StorageState"))?;
    storage.backend.set(&storage.origin, &key, &value);
    Ok(())
}

/// Remove a key from localStorage.
#[op2(fast)]
pub fn op_storage_remove(
    state: Rc<RefCell<OpState>>,
    #[string] key: String,
) -> Result<(), deno_error::JsErrorBox> {
    let s = state.borrow();
    let storage = s
        .try_borrow::<StorageState>()
        .ok_or_else(|| deno_error::JsErrorBox::generic("No StorageState"))?;
    storage.backend.remove(&storage.origin, &key);
    Ok(())
}
