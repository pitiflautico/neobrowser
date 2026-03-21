//! Integration tests for neo-dom.

use neo_dom::{DomEngine, Html5everDom, MockDomEngine};

const TEST_HTML: &str = r#"<!DOCTYPE html>
<html>
<head><title>Test Page</title></head>
<body>
  <h1>Hello World</h1>
  <a href="/about" rel="nofollow">About Us</a>
  <a href="/contact">Contact</a>
  <button aria-label="Submit Form">Submit</button>
  <div hidden>Secret</div>
  <div style="display:none">Invisible</div>
  <div aria-hidden="true">Also hidden</div>
  <input type="text" id="email" placeholder="Enter email" />
  <label for="email">Email Address</label>
  <form id="login" action="/login" method="post">
    <input type="text" name="username" required placeholder="Username" />
    <input type="password" name="password" required />
    <button type="submit">Log In</button>
  </form>
  <div role="button" tabindex="0">Custom Button</div>
</body>
</html>"#;

fn parsed_dom() -> Html5everDom {
    let mut dom = Html5everDom::new();
    dom.parse_html(TEST_HTML, "https://example.com")
        .expect("parse should succeed");
    dom
}

#[test]
fn parse_and_title() {
    let dom = parsed_dom();
    assert_eq!(dom.title(), "Test Page");
}

#[test]
fn get_links() {
    let dom = parsed_dom();
    let links = dom.get_links();
    assert_eq!(links.len(), 2);
    assert_eq!(links[0].href, "/about");
    assert_eq!(links[0].text, "About Us");
    assert_eq!(links[0].rel.as_deref(), Some("nofollow"));
    assert_eq!(links[1].href, "/contact");
}

#[test]
fn query_selector_tag() {
    let dom = parsed_dom();
    let h1 = dom.query_selector("h1");
    assert!(h1.is_some());
    assert_eq!(dom.tag_name(h1.unwrap()).as_deref(), Some("h1"));
    assert_eq!(dom.text_content(h1.unwrap()), "Hello World");
}

#[test]
fn query_selector_by_id() {
    let dom = parsed_dom();
    let el = dom.query_selector("#email");
    assert!(el.is_some());
    assert_eq!(dom.tag_name(el.unwrap()).as_deref(), Some("input"));
}

#[test]
fn query_selector_all() {
    let dom = parsed_dom();
    let inputs = dom.query_selector_all("input");
    // email + username + password = 3
    assert_eq!(inputs.len(), 3);
}

#[test]
fn query_by_text_finds_button() {
    let dom = parsed_dom();
    let el = dom.query_by_text("Submit");
    assert!(el.is_some());
    assert_eq!(dom.tag_name(el.unwrap()).as_deref(), Some("button"));
}

#[test]
fn query_by_text_case_insensitive() {
    let dom = parsed_dom();
    let el = dom.query_by_text("hello world");
    assert!(el.is_some());
}

#[test]
fn query_by_role_button() {
    let dom = parsed_dom();
    let el = dom.query_by_role("button", Some("Submit Form"));
    assert!(el.is_some());
    assert_eq!(dom.tag_name(el.unwrap()).as_deref(), Some("button"));
}

#[test]
fn get_forms_with_fields() {
    let dom = parsed_dom();
    let forms = dom.get_forms();
    assert_eq!(forms.len(), 1);
    let form = &forms[0];
    assert_eq!(form.id.as_deref(), Some("login"));
    assert_eq!(form.action, "/login");
    assert_eq!(form.method, "POST");
    assert_eq!(form.fields.len(), 2); // 2 inputs inside form
    assert_eq!(form.fields[0].name, "username");
    assert!(form.fields[0].required);
    assert_eq!(form.fields[1].field_type, "password");
}

#[test]
fn is_visible_hidden_attr() {
    let dom = parsed_dom();
    let el = dom.query_by_text("Secret");
    assert!(el.is_some());
    assert!(!dom.is_visible(el.unwrap()));
}

#[test]
fn is_visible_display_none() {
    let dom = parsed_dom();
    let el = dom.query_by_text("Invisible");
    assert!(el.is_some());
    assert!(!dom.is_visible(el.unwrap()));
}

#[test]
fn is_visible_aria_hidden() {
    let dom = parsed_dom();
    let el = dom.query_by_text("Also hidden");
    assert!(el.is_some());
    assert!(!dom.is_visible(el.unwrap()));
}

#[test]
fn is_visible_normal_element() {
    let dom = parsed_dom();
    let el = dom.query_selector("h1").unwrap();
    assert!(dom.is_visible(el));
}

#[test]
fn is_interactive() {
    let dom = parsed_dom();
    // input is interactive
    let input = dom.query_selector("#email").unwrap();
    assert!(dom.is_interactive(input));
    // h1 is not
    let h1 = dom.query_selector("h1").unwrap();
    assert!(!dom.is_interactive(h1));
    // div[role=button][tabindex] is interactive
    let custom = dom.query_by_text("Custom Button").unwrap();
    assert!(dom.is_interactive(custom));
}

#[test]
fn accessible_name_aria_label() {
    let dom = parsed_dom();
    let btn = dom.query_by_role("button", Some("Submit Form")).unwrap();
    assert_eq!(dom.accessible_name(btn), "Submit Form");
}

#[test]
fn accessible_name_label_for() {
    let dom = parsed_dom();
    let input = dom.query_selector("#email").unwrap();
    // Should find "Email Address" from label[for=email]
    let name = dom.accessible_name(input);
    assert_eq!(name, "Email Address");
}

#[test]
fn accessible_name_placeholder_fallback() {
    let dom = parsed_dom();
    // username input has no label[for], should fall back to placeholder
    let forms = dom.get_forms();
    let username_input = dom
        .query_selector_all("input")
        .into_iter()
        .find(|&i| dom.get_attribute(i, "name").as_deref() == Some("username"));
    assert!(username_input.is_some());
    let name = dom.accessible_name(username_input.unwrap());
    assert_eq!(name, "Username");
}

#[test]
fn set_attribute_and_read() {
    let mut dom = parsed_dom();
    let el = dom.query_selector("h1").unwrap();
    dom.set_attribute(el, "class", "main-title");
    assert_eq!(
        dom.get_attribute(el, "class").as_deref(),
        Some("main-title")
    );
}

#[test]
fn mock_engine_basic() {
    let mut mock = MockDomEngine::new();
    mock.set_title("Mock Page");
    let btn = mock.add_element("button", &[("aria-label", "Click me")], "Click");
    mock.set_interactive(btn, true);

    assert_eq!(mock.title(), "Mock Page");
    assert_eq!(mock.tag_name(btn).as_deref(), Some("button"));
    assert!(mock.is_interactive(btn));
    assert_eq!(mock.accessible_name(btn), "Click me");
}

#[test]
fn parse_invalid_url() {
    let mut dom = Html5everDom::new();
    let result = dom.parse_html("<html></html>", "not a url");
    assert!(result.is_err());
}
