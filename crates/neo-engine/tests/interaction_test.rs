//! Interaction pipeline tests — click, type, press_key, submit via LiveDom + V8.
//!
//! Uses real V8 runtime (DenoRuntime) + LiveDom to verify the full interaction
//! pipeline. Marked `#[ignore]` because V8 compilation is heavy.
//!
//! Run with: cargo test -p neo-engine --test interaction_test -- --ignored

use neo_engine::{ActionOutcome, LiveDom};
use neo_runtime::v8::DenoRuntime;
use neo_runtime::{JsRuntime, RuntimeConfig};

/// Helper: create a DenoRuntime with HTML and return it ready for LiveDom.
fn setup(html: &str) -> DenoRuntime {
    let mut rt = DenoRuntime::new(&RuntimeConfig::default())
        .expect("failed to create DenoRuntime");
    rt.set_document_html(html, "https://example.com")
        .expect("set_document_html failed");
    rt
}

// ─── Click on button → DomChanged ───────────────────────────────────

#[test]
#[ignore]
fn click_button_dom_changed() {
    let mut rt = setup(r#"<html><body>
        <button id="btn" onclick="document.body.innerHTML += '<p>added</p>'">Click me</button>
    </body></html>"#);
    let mut dom = LiveDom::new(&mut rt);
    let result = dom.click("#btn").unwrap();
    // The onclick handler adds a <p>, so DOM should change.
    assert!(result.mutations > 0 || matches!(result.outcome, ActionOutcome::DomOnlyUpdate { .. }),
        "expected DOM change, got outcome={:?} mutations={}", result.outcome, result.mutations);
}

// ─── Click on link → Navigation ─────────────────────────────────────

#[test]
#[ignore]
fn click_link_navigation() {
    let mut rt = setup(r#"<html><body>
        <a id="link" href="https://example.com/other">Go</a>
    </body></html>"#);
    let mut dom = LiveDom::new(&mut rt);
    let result = dom.click("#link").unwrap();
    // Link click should trigger navigation outcome.
    assert!(
        matches!(result.outcome, ActionOutcome::HttpNavigation { .. }),
        "expected HttpNavigation, got {:?}", result.outcome
    );
}

// ─── Type in input → value changes + events fire ─────────────────────

#[test]
#[ignore]
fn type_in_input_value_changes() {
    let mut rt = setup(r#"<html><body>
        <input id="name" type="text" value="" />
    </body></html>"#);
    let mut dom = LiveDom::new(&mut rt);
    dom.type_text("#name", "hello").unwrap();
    let val = dom.get_value("#name").unwrap();
    assert_eq!(val.value, "hello", "input value should be 'hello' after typing");
}

// ─── Type in email input → no selection error ────────────────────────

#[test]
#[ignore]
fn type_in_email_input_no_error() {
    let mut rt = setup(r#"<html><body>
        <input id="email" type="email" value="" />
    </body></html>"#);
    let mut dom = LiveDom::new(&mut rt);
    // Email inputs throw InvalidStateError on selectionStart access.
    // Our dispatcher should handle this gracefully.
    let result = dom.type_text("#email", "test@example.com");
    assert!(result.is_ok(), "typing in email input should not error: {:?}", result.err());
}

// ─── Press Enter in input → form submit ──────────────────────────────

#[test]
#[ignore]
fn press_enter_submits_form() {
    let mut rt = setup(r#"<html><body>
        <form id="loginform" action="/login" method="POST">
            <input id="user" type="text" name="username" value="admin" />
            <input id="pass" type="password" name="password" value="secret" />
        </form>
    </body></html>"#);
    let mut dom = LiveDom::new(&mut rt);
    let result = dom.press_key("#user", "Enter").unwrap();
    // Enter in a form input should submit.
    let is_nav = matches!(result.outcome, ActionOutcome::HttpNavigation { .. });
    let is_blocked = matches!(result.outcome, ActionOutcome::ValidationBlocked);
    assert!(is_nav || is_blocked,
        "expected HttpNavigation or ValidationBlocked, got {:?}", result.outcome);
}

// ─── Click checkbox → toggle ─────────────────────────────────────────

#[test]
#[ignore]
fn click_checkbox_toggle() {
    let mut rt = setup(r#"<html><body>
        <input id="agree" type="checkbox" />
    </body></html>"#);
    let mut dom = LiveDom::new(&mut rt);
    let result = dom.click("#agree").unwrap();
    assert!(
        matches!(result.outcome, ActionOutcome::CheckboxToggled { checked: true }),
        "expected CheckboxToggled(true), got {:?}", result.outcome
    );
}

// ─── Click disabled button → error ───────────────────────────────────

#[test]
#[ignore]
fn click_disabled_not_interactable() {
    let mut rt = setup(r#"<html><body>
        <button id="btn" disabled>Disabled</button>
    </body></html>"#);
    let mut dom = LiveDom::new(&mut rt);
    let err = dom.click("#btn").unwrap_err();
    assert!(
        matches!(err, neo_engine::LiveDomError::NotInteractable { .. }),
        "expected NotInteractable, got {:?}", err
    );
}

// ─── dispatch_action generic path ────────────────────────────────────

#[test]
#[ignore]
fn dispatch_action_click_button() {
    let mut rt = setup(r#"<html><body>
        <button id="btn" onclick="document.body.innerHTML += '<p>ok</p>'">Go</button>
    </body></html>"#);
    let mut dom = LiveDom::new(&mut rt);
    let result = dom.dispatch_action("click", "#btn", "", "").unwrap();
    assert!(result.mutations > 0 || matches!(result.outcome, ActionOutcome::DomOnlyUpdate { .. }),
        "dispatch_action click should detect DOM change");
}

#[test]
#[ignore]
fn dispatch_action_type_text() {
    let mut rt = setup(r#"<html><body>
        <input id="q" type="text" />
    </body></html>"#);
    let mut dom = LiveDom::new(&mut rt);
    let result = dom.dispatch_action("type_text", "#q", "search term", "").unwrap();
    assert!(matches!(result.outcome, ActionOutcome::ValueChanged | ActionOutcome::DomOnlyUpdate { .. } | ActionOutcome::NoEffect),
        "dispatch_action type_text should succeed, got {:?}", result.outcome);
}

// ─── Fill form (multi-field) ─────────────────────────────────────────

#[test]
#[ignore]
fn fill_form_multiple_fields() {
    let mut rt = setup(r#"<html><body>
        <form>
            <input id="first" type="text" name="first" />
            <input id="last" type="text" name="last" />
        </form>
    </body></html>"#);
    let mut dom = LiveDom::new(&mut rt);
    let result = dom.fill_form(&[("#first", "John"), ("#last", "Doe")]).unwrap();
    // Both fields should have been filled.
    let v1 = dom.get_value("#first").unwrap();
    let v2 = dom.get_value("#last").unwrap();
    assert_eq!(v1.value, "John");
    assert_eq!(v2.value, "Doe");
    assert!(result.mutations >= 0); // sanity
}

// ─── Click element not found ─────────────────────────────────────────

#[test]
#[ignore]
fn click_nonexistent_element() {
    let mut rt = setup(r#"<html><body><p>empty</p></body></html>"#);
    let mut dom = LiveDom::new(&mut rt);
    let err = dom.click("#does-not-exist").unwrap_err();
    assert!(matches!(err, neo_engine::LiveDomError::NotFound(_)));
}
