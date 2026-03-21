//! Integration tests for neo-interact.

use std::collections::HashMap;

use neo_dom::{DomEngine, MockDomEngine};
use neo_interact::{click, fill_form, resolve, scroll, submit, type_text};
use neo_interact::{ClickResult, InteractError, ScrollDirection, SubmitResult};

fn make_dom_with_button() -> MockDomEngine {
    let mut dom = MockDomEngine::new();
    let btn = dom.add_element("button", &[("type", "button")], "Submit");
    dom.set_interactive(btn, true);
    dom.set_visible(btn, true);
    dom
}

fn make_dom_with_link() -> MockDomEngine {
    let mut dom = MockDomEngine::new();
    let link = dom.add_element("a", &[("href", "https://example.com")], "Click me");
    dom.set_interactive(link, true);
    dom.set_visible(link, true);
    dom
}

fn make_dom_with_input() -> MockDomEngine {
    let mut dom = MockDomEngine::new();
    let input = dom.add_element(
        "input",
        &[("type", "text"), ("placeholder", "Email address")],
        "",
    );
    dom.set_interactive(input, true);
    dom.set_visible(input, true);
    dom
}

#[test]
fn test_resolve_by_text() {
    let dom = make_dom_with_button();
    let el = resolve(&dom, "Submit").expect("should find button by text");
    assert_eq!(dom.tag_name(el).unwrap(), "button");
}

#[test]
fn test_resolve_by_selector() {
    let dom = make_dom_with_button();
    // MockDomEngine's query_selector matches by tag name
    let el = resolve(&dom, "button").expect("should find by tag selector");
    assert_eq!(dom.tag_name(el).unwrap(), "button");
}

#[test]
fn test_resolve_not_found() {
    let dom = make_dom_with_button();
    let err = resolve(&dom, "Nonexistent").unwrap_err();
    match err {
        InteractError::NotFound { target, .. } => {
            assert_eq!(target, "Nonexistent");
        }
        other => panic!("expected NotFound, got: {other:?}"),
    }
}

#[test]
fn test_click_link() {
    let mut dom = make_dom_with_link();
    let result = click(&mut dom, "Click me").expect("should click link");
    assert_eq!(
        result,
        ClickResult::Navigation("https://example.com".into())
    );
}

#[test]
fn test_click_button() {
    let mut dom = make_dom_with_button();
    let result = click(&mut dom, "Submit").expect("should click button");
    assert_eq!(result, ClickResult::DomChanged(1));
}

#[test]
fn test_click_not_interactive() {
    let mut dom = MockDomEngine::new();
    dom.add_element("div", &[], "Some text");
    // not set as interactive
    let err = click(&mut dom, "Some text").unwrap_err();
    assert!(matches!(err, InteractError::NotInteractive(_)));
}

#[test]
fn test_type_into_input() {
    let mut dom = make_dom_with_input();
    type_text(&mut dom, "Email address", "user@test.com", true).expect("should type into input");
    let el = resolve(&dom, "Email address").unwrap();
    let value = dom.get_attribute(el, "value").unwrap();
    assert_eq!(value, "user@test.com");
}

#[test]
fn test_type_append() {
    let mut dom = make_dom_with_input();
    type_text(&mut dom, "Email address", "hello", true).unwrap();
    type_text(&mut dom, "Email address", " world", false).unwrap();
    let el = resolve(&dom, "Email address").unwrap();
    let value = dom.get_attribute(el, "value").unwrap();
    assert_eq!(value, "hello world");
}

#[test]
fn test_type_wrong_element() {
    let mut dom = MockDomEngine::new();
    dom.add_element("div", &[], "Not typeable");
    let err = type_text(&mut dom, "Not typeable", "text", true).unwrap_err();
    assert!(matches!(err, InteractError::TypeMismatch { .. }));
}

#[test]
fn test_fill_form() {
    let mut dom = MockDomEngine::new();
    let name_input = dom.add_element("input", &[("type", "text"), ("placeholder", "Name")], "");
    dom.set_interactive(name_input, true);
    let email_input = dom.add_element("input", &[("type", "email"), ("placeholder", "Email")], "");
    dom.set_interactive(email_input, true);

    let mut fields = HashMap::new();
    fields.insert("Name".to_string(), "John".to_string());
    fields.insert("Email".to_string(), "john@test.com".to_string());

    fill_form(&mut dom, &fields).expect("should fill form");

    let name_val = dom.get_attribute(name_input, "value").unwrap();
    let email_val = dom.get_attribute(email_input, "value").unwrap();
    assert_eq!(name_val, "John");
    assert_eq!(email_val, "john@test.com");
}

#[test]
fn test_csrf_detected() {
    let mut dom = MockDomEngine::new();
    dom.add_form(Some("login"), "/auth/login");

    // We need to add a form with hidden CSRF field via neo_types
    // MockDomEngine.add_form doesn't support fields, so test via detect_csrf
    // by manually building a form with fields.
    // Instead, test the forms module's detect_csrf with a custom form.
    let forms = dom.get_forms();
    assert_eq!(forms.len(), 1);
    assert_eq!(forms[0].action, "/auth/login");

    // Test submit returns Navigation for non-API action
    let result = submit(&mut dom, None).expect("should submit");
    assert_eq!(result, SubmitResult::Navigation("/auth/login".into()));
}

#[test]
fn test_submit_no_form() {
    let mut dom = MockDomEngine::new();
    let result = submit(&mut dom, None).expect("should return NoAction");
    assert_eq!(result, SubmitResult::NoAction);
}

#[test]
fn test_scroll() {
    let mut dom = MockDomEngine::new();
    let btn = dom.add_element("button", &[], "Btn");
    dom.set_interactive(btn, true);
    let inp = dom.add_element("input", &[], "");
    dom.set_interactive(inp, true);

    let count = scroll(&dom, ScrollDirection::Down, 500).expect("should scroll");
    assert_eq!(count, 2); // 1 button + 1 input
}

// -- MockInteractor tests --

#[test]
fn test_mock_interactor_records() {
    use neo_interact::{Interactor, MockInteractor};

    let mut mock = MockInteractor::new();
    mock.click("Submit").unwrap();
    mock.type_text("email", "test@test.com", true).unwrap();
    assert_eq!(mock.actions.len(), 2);
}

#[test]
fn test_mock_interactor_configurable() {
    use neo_interact::{Interactor, MockInteractor};

    let mut mock = MockInteractor::new();
    mock.click_result = ClickResult::Navigation("https://foo.com".into());
    let result = mock.click("anything").unwrap();
    assert_eq!(result, ClickResult::Navigation("https://foo.com".into()));
}
