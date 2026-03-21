//! Integration tests for neo-extract.

use neo_dom::{DomEngine, Html5everDom};
use neo_extract::*;

/// Helper: parse HTML into a Dom engine.
fn dom_from(html: &str) -> Html5everDom {
    let mut dom = Html5everDom::new();
    dom.parse_html(html, "https://example.com").unwrap();
    dom
}

// -- WOM tests --

#[test]
fn test_wom_basic() {
    let html = r#"<html><body>
        <button>Save</button>
        <a href="/about">About</a>
    </body></html>"#;
    let dom = dom_from(html);
    let wom = neo_extract::wom::build_wom(&dom, "https://example.com");

    // Should have at least a button node and a link node
    let button = wom.nodes.iter().find(|n| n.role == "button");
    assert!(button.is_some(), "should find button node");
    assert!(button.unwrap().actions.contains(&"click".to_string()));

    let link = wom.nodes.iter().find(|n| n.role == "link");
    assert!(link.is_some(), "should find link node");
    assert!(link.unwrap().actions.contains(&"click".to_string()));
}

#[test]
fn test_wom_form() {
    let html = r#"<html><body>
        <form action="/login">
            <input type="text" name="user" placeholder="Username">
            <input type="password" name="pass">
            <input type="submit" value="Login">
        </form>
    </body></html>"#;
    let dom = dom_from(html);
    let wom = neo_extract::wom::build_wom(&dom, "https://example.com");

    // Text input should have "type" action
    let text_inputs: Vec<_> = wom
        .nodes
        .iter()
        .filter(|n| n.tag == "input" && n.actions.contains(&"type".to_string()))
        .collect();
    assert!(!text_inputs.is_empty(), "should have typeable inputs");

    // Submit input should have "click" action
    let submits: Vec<_> = wom
        .nodes
        .iter()
        .filter(|n| n.tag == "input" && n.actions.contains(&"click".to_string()))
        .collect();
    assert!(!submits.is_empty(), "should have submit button");

    // Form node should have "submit" action
    let form = wom.nodes.iter().find(|n| n.tag == "form");
    assert!(form.is_some(), "should have form node");
    assert!(form.unwrap().actions.contains(&"submit".to_string()));
}

#[test]
fn test_wom_stable_ids() {
    let html = r#"<html><body>
        <button>Save</button>
        <a href="/about">About</a>
    </body></html>"#;
    let dom1 = dom_from(html);
    let dom2 = dom_from(html);

    let wom1 = neo_extract::wom::build_wom(&dom1, "https://example.com");
    let wom2 = neo_extract::wom::build_wom(&dom2, "https://example.com");

    let ids1: Vec<&str> = wom1.nodes.iter().map(|n| n.id.as_str()).collect();
    let ids2: Vec<&str> = wom2.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids1, ids2, "same HTML should produce same IDs");
}

// -- Structured data tests --

#[test]
fn test_structured_table() {
    let html = r#"<html><body>
        <table>
            <tr><th>Name</th><th>Price</th></tr>
            <tr><td>Widget</td><td>$10</td></tr>
            <tr><td>Gadget</td><td>$20</td></tr>
        </table>
    </body></html>"#;
    let dom = dom_from(html);
    let data = neo_extract::structured::extract_structured(&dom);

    let table = data
        .iter()
        .find(|d| matches!(d, StructuredData::Table { .. }));
    assert!(table.is_some(), "should extract table");

    if let Some(StructuredData::Table { headers, rows }) = table {
        assert_eq!(headers, &["Name", "Price"]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], vec!["Widget", "$10"]);
    }
}

#[test]
fn test_structured_jsonld() {
    let html = r#"<html><head>
        <script type="application/ld+json">
        {
            "@type": "Product",
            "name": "Cool Widget",
            "url": "https://example.com/widget",
            "offers": { "price": "29.99" }
        }
        </script>
    </head><body></body></html>"#;
    let dom = dom_from(html);
    let data = neo_extract::structured::extract_structured(&dom);

    let product = data
        .iter()
        .find(|d| matches!(d, StructuredData::Product { .. }));
    assert!(product.is_some(), "should extract JSON-LD product");

    if let Some(StructuredData::Product { name, price, url }) = product {
        assert_eq!(name, "Cool Widget");
        assert!(price.is_some());
        assert_eq!(url.as_deref(), Some("https://example.com/widget"));
    }
}

// -- Classification tests --

#[test]
fn test_classify_login() {
    let html = r#"<html><body>
        <form action="/auth">
            <input type="text" name="user">
            <input type="password" name="pass">
            <button type="submit">Login</button>
        </form>
    </body></html>"#;
    let dom = dom_from(html);
    let result = neo_extract::classify::classify(&dom);

    assert_eq!(result.page_type, PageType::LoginForm);
    assert!(result.confidence > 0.5);
}

#[test]
fn test_classify_article() {
    let html = r#"<html><body>
        <article>
            <h1>Big News</h1>
            <p>First paragraph of the story.</p>
            <p>Second paragraph continues.</p>
            <p>Third paragraph wraps up.</p>
            <p>Fourth paragraph conclusion.</p>
        </article>
    </body></html>"#;
    let dom = dom_from(html);
    let result = neo_extract::classify::classify(&dom);

    assert_eq!(result.page_type, PageType::Article);
    assert!(result.confidence > 0.5);
}

// -- Delta tests --

#[test]
fn test_delta() {
    let html1 = r#"<html><body>
        <button>Save</button>
    </body></html>"#;
    let html2 = r#"<html><body>
        <button>Save</button>
        <button>Delete</button>
    </body></html>"#;

    let dom1 = dom_from(html1);
    let dom2 = dom_from(html2);

    let wom1 = neo_extract::wom::build_wom(&dom1, "https://example.com");
    let wom2 = neo_extract::wom::build_wom(&dom2, "https://example.com");

    let delta = neo_extract::delta::compute_delta(&wom1, &wom2);

    assert!(
        !delta.added.is_empty(),
        "should detect added button: {:?}",
        delta
    );
    assert!(
        delta.summary.contains("added"),
        "summary should mention additions"
    );
}
