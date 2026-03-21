//! Synthetic fixture validation — verifies the extraction pipeline on realistic HTML.
//!
//! Tests the full path: html5ever parse → DomEngine → classify + WOM + structured extraction.
//! No V8 runtime needed — pure Rust DOM parsing.

use neo_dom::{DomEngine, Html5everDom};
use neo_extract::classify::PageType;
use neo_extract::{DefaultExtractor, Extractor, StructuredData};

/// Fixtures live at the workspace root: tests/fixtures/*.html
const FIXTURES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

/// Helper: load fixture file and parse into DomEngine.
fn load_fixture(name: &str) -> Html5everDom {
    let path = format!("{}/{}", FIXTURES_DIR, name);
    let html = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {}", path, e));
    let mut dom = Html5everDom::new();
    dom.parse_html(&html, "https://example.com")
        .unwrap_or_else(|e| panic!("failed to parse fixture {}: {}", name, e));
    dom
}

// ── basic_page.html ──────────────────────────────────────────────────────

#[test]
fn basic_page_classified_as_article() {
    let dom = load_fixture("basic_page.html");
    let ext = DefaultExtractor::new();
    let classification = ext.classify(&dom);

    assert_eq!(
        classification.page_type,
        PageType::Article,
        "basic_page should be classified as Article, got {:?} (features: {:?})",
        classification.page_type,
        classification.features,
    );
    assert!(
        classification.confidence >= 0.5,
        "article confidence should be >= 0.5, got {}",
        classification.confidence
    );
}

#[test]
fn basic_page_has_links() {
    let dom = load_fixture("basic_page.html");
    let links = dom.get_links();
    assert!(
        links.len() >= 4,
        "basic_page should have at least 4 links, got {}",
        links.len()
    );
}

#[test]
fn basic_page_has_text_content() {
    let dom = load_fixture("basic_page.html");
    let ext = DefaultExtractor::new();
    let semantic = ext.semantic_text(&dom, 10000);
    assert!(
        semantic.len() > 100,
        "basic_page semantic text should be > 100 chars, got {}",
        semantic.len()
    );
}

#[test]
fn basic_page_wom_has_nodes() {
    let dom = load_fixture("basic_page.html");
    let ext = DefaultExtractor::new();
    let wom = ext.extract_wom(&dom);

    assert!(!wom.nodes.is_empty(), "WOM should have nodes");

    let link_nodes: Vec<_> = wom.nodes.iter().filter(|n| n.role == "link").collect();
    assert!(
        link_nodes.len() >= 4,
        "WOM should have at least 4 link nodes, got {}",
        link_nodes.len()
    );
}

// ── login_form.html ──────────────────────────────────────────────────────

#[test]
fn login_form_classified_as_login() {
    let dom = load_fixture("login_form.html");
    let ext = DefaultExtractor::new();
    let classification = ext.classify(&dom);

    assert_eq!(
        classification.page_type,
        PageType::LoginForm,
        "login_form should be classified as LoginForm, got {:?} (features: {:?})",
        classification.page_type,
        classification.features,
    );
}

#[test]
fn login_form_has_inputs() {
    let dom = load_fixture("login_form.html");
    let inputs = dom.get_inputs();
    // email + password + checkbox + hidden csrf = 4 inputs
    assert!(
        inputs.len() >= 2,
        "login_form should have at least 2 inputs, got {}",
        inputs.len()
    );

    // Verify password input exists
    let has_password = inputs
        .iter()
        .any(|&el| dom.get_attribute(el, "type").as_deref() == Some("password"));
    assert!(has_password, "login_form should have a password input");
}

#[test]
fn login_form_has_submit_button() {
    let dom = load_fixture("login_form.html");
    let buttons = dom.get_buttons();
    assert!(
        !buttons.is_empty(),
        "login_form should have at least one button"
    );
}

#[test]
fn login_form_has_csrf_token() {
    let dom = load_fixture("login_form.html");
    let csrf = dom.query_selector("input[name='_csrf']");
    assert!(csrf.is_some(), "login_form should have a CSRF token input");
}

#[test]
fn login_form_has_form() {
    let dom = load_fixture("login_form.html");
    let forms = dom.get_forms();
    assert!(
        !forms.is_empty(),
        "login_form should detect at least one form"
    );
}

// ── data_table.html ──────────────────────────────────────────────────────

#[test]
fn data_table_classified_as_data_table() {
    let dom = load_fixture("data_table.html");
    let ext = DefaultExtractor::new();
    let classification = ext.classify(&dom);

    assert_eq!(
        classification.page_type,
        PageType::DataTable,
        "data_table should be classified as DataTable, got {:?} (features: {:?})",
        classification.page_type,
        classification.features,
    );
}

#[test]
fn data_table_structured_extraction() {
    let dom = load_fixture("data_table.html");
    let ext = DefaultExtractor::new();
    let structured = ext.extract_structured(&dom);

    let tables: Vec<_> = structured
        .iter()
        .filter(|s| matches!(s, StructuredData::Table { .. }))
        .collect();
    assert!(
        !tables.is_empty(),
        "data_table should extract at least one table"
    );

    if let StructuredData::Table { headers, rows } = &tables[0] {
        assert!(
            headers.len() >= 4,
            "table should have at least 4 headers, got {}",
            headers.len()
        );
        assert!(
            rows.len() >= 5,
            "table should have at least 5 data rows, got {}",
            rows.len()
        );
    }
}

// ── search_page.html ─────────────────────────────────────────────────────

#[test]
fn search_page_classified_as_search_results() {
    let dom = load_fixture("search_page.html");
    let ext = DefaultExtractor::new();
    let classification = ext.classify(&dom);

    assert_eq!(
        classification.page_type,
        PageType::SearchResults,
        "search_page should be classified as SearchResults, got {:?} (features: {:?})",
        classification.page_type,
        classification.features,
    );
}

#[test]
fn search_page_has_many_links() {
    let dom = load_fixture("search_page.html");
    let links = dom.get_links();
    assert!(
        links.len() > 10,
        "search_page should have > 10 links (result links), got {}",
        links.len()
    );
}

#[test]
fn search_page_has_search_input() {
    let dom = load_fixture("search_page.html");
    let inputs = dom.get_inputs();
    let has_search = inputs.iter().any(|&el| {
        dom.get_attribute(el, "type").as_deref() == Some("search")
    });
    assert!(has_search, "search_page should have a search input");
}

// ── react_ssr.html ───────────────────────────────────────────────────────

#[test]
fn react_ssr_has_hydration_markers() {
    let dom = load_fixture("react_ssr.html");

    // data-reactroot attribute on root div
    let root = dom.query_selector("[data-reactroot]");
    assert!(root.is_some(), "react_ssr should have data-reactroot element");

    // data-react-hydrate marker
    let hydrate = dom.query_selector("[data-react-hydrate]");
    assert!(hydrate.is_some(), "react_ssr should have data-react-hydrate marker");
}

#[test]
fn react_ssr_has_react_router_manifest() {
    let dom = load_fixture("react_ssr.html");
    let html = dom.outer_html();
    assert!(
        html.contains("__reactRouterManifest"),
        "react_ssr should contain __reactRouterManifest in a script"
    );
}

#[test]
fn react_ssr_has_navigable_content() {
    let dom = load_fixture("react_ssr.html");
    let links = dom.get_links();
    assert!(
        links.len() >= 3,
        "react_ssr (SSR) should have at least 3 nav links, got {}",
        links.len()
    );
}

// ── spa_shell.html ───────────────────────────────────────────────────────

#[test]
fn spa_shell_has_empty_root() {
    // Without JS execution, the root div should be empty.
    // This validates that html5ever parsing alone does NOT execute JS.
    let dom = load_fixture("spa_shell.html");
    let root = dom.query_selector("#root");
    assert!(root.is_some(), "spa_shell should have #root element");

    let root_el = root.expect("root should exist");
    let inner = dom.inner_html(root_el);
    // html5ever does NOT execute JS, so #root should be empty
    assert!(
        inner.trim().is_empty(),
        "spa_shell #root should be empty without JS execution, got: '{}'",
        inner
    );
}

#[test]
fn spa_shell_has_noscript_fallback() {
    let dom = load_fixture("spa_shell.html");
    let noscript = dom.query_selector("noscript");
    assert!(noscript.is_some(), "spa_shell should have a noscript element");
}

// ── consent_banner.html ──────────────────────────────────────────────────

#[test]
fn consent_banner_detects_dialog() {
    let dom = load_fixture("consent_banner.html");
    let dialog = neo_interact::detect_modal(&dom);
    assert!(dialog.is_some(), "consent_banner should detect a modal dialog");
}

#[test]
fn consent_banner_has_accept_button() {
    let dom = load_fixture("consent_banner.html");
    let accept = dom.query_by_text("Accept");
    assert!(
        accept.is_some(),
        "consent_banner should have an 'Accept' button"
    );
    let aceptar = dom.query_by_text("Aceptar");
    assert!(
        aceptar.is_some(),
        "consent_banner should have an 'Aceptar' button"
    );
}

#[test]
fn consent_banner_dismiss_works() {
    let mut dom = load_fixture("consent_banner.html");
    let dismissed = neo_interact::dismiss_consent(&mut dom);
    assert!(dismissed, "dismiss_consent should find and click a consent button");
}

// ── interactive_page.html ────────────────────────────────────────────────

#[test]
fn interactive_page_counts_inputs() {
    let dom = load_fixture("interactive_page.html");
    let inputs = dom.get_inputs();
    // text + email + 3 checkboxes + 3 radios + file = 9 (textarea/select are separate)
    assert!(
        inputs.len() >= 9,
        "interactive_page should have at least 9 inputs, got {}",
        inputs.len()
    );
}

#[test]
fn interactive_page_counts_buttons() {
    let dom = load_fixture("interactive_page.html");
    let buttons = dom.get_buttons();
    // submit + reset = 2
    assert!(
        buttons.len() >= 2,
        "interactive_page should have at least 2 buttons, got {}",
        buttons.len()
    );
}

#[test]
fn interactive_page_wom_interactive_elements() {
    let dom = load_fixture("interactive_page.html");
    let ext = DefaultExtractor::new();
    let wom = ext.extract_wom(&dom);

    let interactive: Vec<_> = wom.nodes.iter().filter(|n| n.interactive).collect();
    assert!(
        interactive.len() >= 10,
        "interactive_page WOM should have at least 10 interactive nodes, got {}",
        interactive.len()
    );
}

#[test]
fn interactive_page_has_file_input() {
    let dom = load_fixture("interactive_page.html");
    let inputs = dom.get_inputs();
    let has_file = inputs
        .iter()
        .any(|&el| dom.get_attribute(el, "type").as_deref() == Some("file"));
    assert!(has_file, "interactive_page should have a file input");
}

#[test]
fn interactive_page_has_links() {
    let dom = load_fixture("interactive_page.html");
    let links = dom.get_links();
    assert!(
        links.len() >= 2,
        "interactive_page should have at least 2 links, got {}",
        links.len()
    );
}

#[test]
fn interactive_page_has_select() {
    let dom = load_fixture("interactive_page.html");
    let select = dom.query_selector("select");
    assert!(select.is_some(), "interactive_page should have a select element");
}

// ── form_complete.html — WOM enrichment fields ─────────────────────────

fn form_complete_wom() -> neo_extract::WomDocument {
    let dom = load_fixture("form_complete.html");
    let ext = DefaultExtractor::new();
    ext.extract_wom(&dom)
}

fn find_node_by_name<'a>(wom: &'a neo_extract::WomDocument, name: &str) -> &'a neo_extract::WomNode {
    wom.nodes
        .iter()
        .find(|n| n.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("WOM node with name='{}' not found", name))
}

#[test]
fn form_complete_email_field() {
    let wom = form_complete_wom();
    let email = find_node_by_name(&wom, "email");
    assert_eq!(email.input_type.as_deref(), Some("email"));
    assert!(email.required, "email should be required");
    assert_eq!(email.placeholder.as_deref(), Some("you@example.com"));
    assert_eq!(email.autocomplete.as_deref(), Some("email"));
    assert!(!email.disabled);
    assert!(!email.readonly);
}

#[test]
fn form_complete_password_field() {
    let wom = form_complete_wom();
    let pw = find_node_by_name(&wom, "password");
    assert_eq!(pw.input_type.as_deref(), Some("password"));
    assert!(pw.required);
    assert_eq!(pw.minlength, Some(8));
    assert_eq!(pw.maxlength, Some(100));
    assert_eq!(pw.placeholder.as_deref(), Some("Password"));
}

#[test]
fn form_complete_phone_field() {
    let wom = form_complete_wom();
    let phone = find_node_by_name(&wom, "phone");
    assert_eq!(phone.pattern.as_deref(), Some("[0-9]{10}"));
    assert_eq!(phone.placeholder.as_deref(), Some("Phone"));
    assert!(!phone.required);
}

#[test]
fn form_complete_age_field() {
    let wom = form_complete_wom();
    let age = find_node_by_name(&wom, "age");
    assert_eq!(age.input_type.as_deref(), Some("number"));
    assert_eq!(age.min.as_deref(), Some("18"));
    assert_eq!(age.max.as_deref(), Some("120"));
}

#[test]
fn form_complete_checkbox_terms() {
    let wom = form_complete_wom();
    let terms = find_node_by_name(&wom, "terms");
    assert_eq!(terms.input_type.as_deref(), Some("checkbox"));
    assert_eq!(terms.checked, Some(false), "terms checkbox has no checked attr");
    assert!(terms.required);
}

#[test]
fn form_complete_radio_basic() {
    let wom = form_complete_wom();
    // Find the radio with value="basic"
    let basic = wom
        .nodes
        .iter()
        .find(|n| n.name.as_deref() == Some("plan") && n.value.as_deref() == Some("basic"))
        .expect("radio plan=basic not found");
    assert_eq!(basic.input_type.as_deref(), Some("radio"));
    assert_eq!(basic.checked, Some(true), "basic radio should be checked");
}

#[test]
fn form_complete_radio_pro() {
    let wom = form_complete_wom();
    let pro = wom
        .nodes
        .iter()
        .find(|n| n.name.as_deref() == Some("plan") && n.value.as_deref() == Some("pro"))
        .expect("radio plan=pro not found");
    assert_eq!(pro.checked, Some(false), "pro radio should not be checked");
}

#[test]
fn form_complete_select_country() {
    let wom = form_complete_wom();
    let country = find_node_by_name(&wom, "country");
    assert_eq!(country.tag, "select");
    assert_eq!(country.options.len(), 3, "country select should have 3 options");
    assert_eq!(country.options[0].value, "ES");
    assert_eq!(country.options[0].text, "Spain");
    assert!(country.options[0].selected, "ES should be selected");
    assert!(!country.options[1].selected, "US should not be selected");
    assert!(!country.options[2].selected, "UK should not be selected");
}

#[test]
fn form_complete_textarea_bio() {
    let wom = form_complete_wom();
    let bio = find_node_by_name(&wom, "bio");
    assert_eq!(bio.tag, "textarea");
    assert_eq!(bio.placeholder.as_deref(), Some("Tell us about yourself"));
    assert_eq!(bio.maxlength, Some(500));
}

#[test]
fn form_complete_readonly_field() {
    let wom = form_complete_wom();
    let ro = find_node_by_name(&wom, "readonly_field");
    assert!(ro.readonly, "readonly_field should have readonly=true");
    assert!(!ro.disabled);
}

#[test]
fn form_complete_disabled_field() {
    let wom = form_complete_wom();
    let dis = find_node_by_name(&wom, "disabled_field");
    assert!(dis.disabled, "disabled_field should have disabled=true");
    assert!(!dis.readonly);
}

#[test]
fn form_complete_submit_button() {
    let wom = form_complete_wom();
    let btn = wom
        .nodes
        .iter()
        .find(|n| n.tag == "button" && n.input_type.as_deref() == Some("submit"))
        .expect("submit button not found");
    assert_eq!(btn.label, "Sign Up");
}
