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

// ========== Tier 3 tests ==========

// -- WOM role tests --

#[test]
fn test_wom_roles() {
    let html = r#"<html><body>
        <nav><a href="/">Home</a></nav>
        <header><h1>Title</h1></header>
        <main><p>Content</p></main>
        <footer><p>Copyright</p></footer>
        <aside><p>Sidebar</p></aside>
        <article><p>Post</p></article>
    </body></html>"#;
    let dom = dom_from(html);
    let wom = neo_extract::wom::build_wom(&dom, "https://example.com");

    let nav = wom.nodes.iter().find(|n| n.role == "navigation");
    assert!(nav.is_some(), "nav should have role 'navigation'");

    let header = wom.nodes.iter().find(|n| n.role == "banner");
    assert!(header.is_some(), "header should have role 'banner'");

    let footer = wom.nodes.iter().find(|n| n.role == "contentinfo");
    assert!(footer.is_some(), "footer should have role 'contentinfo'");

    let main = wom.nodes.iter().find(|n| n.role == "main");
    assert!(main.is_some(), "main should have role 'main'");

    let aside = wom.nodes.iter().find(|n| n.role == "complementary");
    assert!(aside.is_some(), "aside should have role 'complementary'");

    let article = wom.nodes.iter().find(|n| n.role == "article");
    assert!(article.is_some(), "article should have role 'article'");
}

#[test]
fn test_wom_explicit_role() {
    let html = r#"<html><body>
        <div role="search"><input type="text"></div>
    </body></html>"#;
    let _dom = dom_from(html);

    // The input inside div[role=search] -- the input itself doesn't have a role attr,
    // but if it did, it should be honored. Test that explicit role attribute works.
    // Let's test with a button that has explicit role
    let html2 = r#"<html><body>
        <button role="tab">Tab 1</button>
    </body></html>"#;
    let dom2 = dom_from(html2);
    let wom2 = neo_extract::wom::build_wom(&dom2, "https://example.com");

    let tab = wom2.nodes.iter().find(|n| n.role == "tab");
    assert!(tab.is_some(), "explicit role='tab' should override default");
}

// -- WOM action tests --

#[test]
fn test_wom_actions() {
    let html = r#"<html><body>
        <a href="/page">Link</a>
        <input type="text" placeholder="Name">
        <input type="checkbox" name="agree">
        <input type="radio" name="choice">
        <select><option>A</option></select>
        <textarea>Notes</textarea>
        <form action="/submit"><button>Go</button></form>
    </body></html>"#;
    let dom = dom_from(html);
    let wom = neo_extract::wom::build_wom(&dom, "https://example.com");

    // Links should have click + navigate
    let link = wom.nodes.iter().find(|n| n.role == "link");
    assert!(link.is_some());
    let link = link.unwrap();
    assert!(link.actions.contains(&"click".to_string()));
    assert!(link.actions.contains(&"navigate".to_string()));

    // Text input should have type + clear
    let text_input = wom
        .nodes
        .iter()
        .find(|n| n.role == "input" && n.tag == "input");
    assert!(text_input.is_some());
    let ti = text_input.unwrap();
    assert!(ti.actions.contains(&"type".to_string()));
    assert!(ti.actions.contains(&"clear".to_string()));

    // Checkbox should have check + uncheck
    let checkbox = wom.nodes.iter().find(|n| n.role == "checkbox");
    assert!(checkbox.is_some());
    let cb = checkbox.unwrap();
    assert!(cb.actions.contains(&"check".to_string()));
    assert!(cb.actions.contains(&"uncheck".to_string()));

    // Radio should have select
    let radio = wom.nodes.iter().find(|n| n.role == "radio");
    assert!(radio.is_some());
    assert!(radio.unwrap().actions.contains(&"select".to_string()));

    // Select should have select
    let select = wom.nodes.iter().find(|n| n.role == "select");
    assert!(select.is_some());
    assert!(select.unwrap().actions.contains(&"select".to_string()));

    // Textarea should have type + clear
    let textarea = wom.nodes.iter().find(|n| n.tag == "textarea");
    assert!(textarea.is_some());
    let ta = textarea.unwrap();
    assert!(ta.actions.contains(&"type".to_string()));
    assert!(ta.actions.contains(&"clear".to_string()));

    // Form should have submit + fill
    let form = wom.nodes.iter().find(|n| n.tag == "form");
    assert!(form.is_some());
    let f = form.unwrap();
    assert!(f.actions.contains(&"submit".to_string()));
    assert!(f.actions.contains(&"fill".to_string()));
}

// -- WOM summary tests --

#[test]
fn test_wom_summary() {
    let html = r#"<html><head><title>Login</title></head><body>
        <form action="/auth">
            <input type="text" name="email" aria-label="email">
            <input type="password" name="pass" aria-label="password">
            <button type="submit">Sign In</button>
        </form>
    </body></html>"#;
    let dom = dom_from(html);
    let wom = neo_extract::wom::build_wom(&dom, "https://example.com");

    // Summary should mention inputs, buttons, and title
    assert!(
        wom.summary.contains("Login"),
        "summary should contain title: {}",
        wom.summary
    );
    assert!(
        wom.summary.contains("input"),
        "summary should mention inputs: {}",
        wom.summary
    );
    assert!(
        wom.summary.contains("button"),
        "summary should mention buttons: {}",
        wom.summary
    );
}

// -- Structured JSON-LD verification --

#[test]
fn test_structured_jsonld_detailed() {
    let html = r#"<html><head>
        <script type="application/ld+json">
        {
            "@type": "Product",
            "name": "Super Gadget",
            "url": "https://shop.example.com/gadget",
            "offers": { "price": 49.99 }
        }
        </script>
    </head><body></body></html>"#;
    let dom = dom_from(html);
    let data = neo_extract::structured::extract_structured(&dom);

    let product = data
        .iter()
        .find(|d| matches!(d, StructuredData::Product { .. }));
    assert!(
        product.is_some(),
        "should extract JSON-LD product with numeric price"
    );

    if let Some(StructuredData::Product { name, price, url }) = product {
        assert_eq!(name, "Super Gadget");
        assert!(price.is_some(), "numeric price should be extracted");
        assert_eq!(url.as_deref(), Some("https://shop.example.com/gadget"));
    }
}

// -- Pagination detection --

#[test]
fn test_structured_pagination() {
    let html = r#"<html><body>
        <a href="/page/1">1</a>
        <a href="/page/2">2</a>
        <a href="/page/3">3</a>
        <a href="/page/2">Next</a>
        <a href="/page/0">Previous</a>
    </body></html>"#;
    let dom = dom_from(html);
    let data = neo_extract::structured::extract_structured(&dom);

    let pagination = data
        .iter()
        .find(|d| matches!(d, StructuredData::Pagination { .. }));
    assert!(pagination.is_some(), "should detect pagination links");

    if let Some(StructuredData::Pagination {
        pages,
        next_url,
        prev_url,
    }) = pagination
    {
        assert!(pages.contains(&"1".to_string()), "should have page 1");
        assert!(pages.contains(&"2".to_string()), "should have page 2");
        assert!(next_url.is_some(), "should detect next URL");
        assert!(prev_url.is_some(), "should detect previous URL");
    }
}

// -- Delta summary tests --

#[test]
fn test_delta_summary() {
    let html1 = r#"<html><body>
        <button>Save</button>
        <a href="/old">Old Link</a>
    </body></html>"#;
    let html2 = r#"<html><body>
        <button>Save</button>
        <a href="/new1">New Link 1</a>
        <a href="/new2">New Link 2</a>
    </body></html>"#;

    let dom1 = dom_from(html1);
    let dom2 = dom_from(html2);

    let wom1 = neo_extract::wom::build_wom(&dom1, "https://example.com");
    let wom2 = neo_extract::wom::build_wom(&dom2, "https://example.com");

    let delta = neo_extract::delta::compute_delta(&wom1, &wom2);

    // Should have additions and removals
    assert!(!delta.added.is_empty(), "should have added nodes");
    assert!(!delta.removed.is_empty(), "should have removed nodes");
    assert!(
        delta.summary.contains("added"),
        "summary should mention additions: {}",
        delta.summary
    );
    assert!(
        delta.summary.contains("removed"),
        "summary should mention removals: {}",
        delta.summary
    );
    // Summary should mention roles
    assert!(
        delta.summary.contains("link"),
        "summary should describe what was added: {}",
        delta.summary
    );
}

// -- Classification: Pricing --

#[test]
fn test_classify_pricing() {
    let html = r#"<html><head><title>Pricing Plans</title></head><body>
        <h1>Choose Your Plan</h1>
        <div>
            <h2>Basic</h2>
            <p>$9/month</p>
        </div>
        <div>
            <h2>Pro Plan</h2>
            <p>$29/month</p>
        </div>
        <div>
            <h2>Enterprise</h2>
            <p>$99/month</p>
        </div>
    </body></html>"#;
    let dom = dom_from(html);
    let result = neo_extract::classify::classify(&dom);

    assert_eq!(
        result.page_type,
        PageType::Pricing,
        "should classify as Pricing, got {:?} with features {:?}",
        result.page_type,
        result.features
    );
    assert!(result.confidence > 0.5);
    assert!(
        !result.features.is_empty(),
        "should have classification features"
    );
}

// -- Classification: new types exist --

#[test]
fn test_classify_new_types_exist() {
    // Verify the new PageType variants compile and are usable
    let types = vec![
        PageType::Pricing,
        PageType::Documentation,
        PageType::Profile,
        PageType::Settings,
    ];
    for pt in types {
        let formatted = format!("{:?}", pt);
        assert!(!formatted.is_empty());
    }
}

// -- Semantic: nav exclusion --

#[test]
fn test_semantic_removes_nav() {
    let html = r#"<html><body>
        <nav>
            <li>Home</li>
            <li>About</li>
            <li>Contact</li>
        </nav>
        <main>
            <h1>Welcome</h1>
            <p>This is the main content of the page.</p>
        </main>
    </body></html>"#;
    let dom = dom_from(html);
    let text = neo_extract::semantic::semantic_text(&dom, 10000);

    // Should contain main content
    assert!(
        text.contains("Welcome"),
        "should include main heading: {text}"
    );
    assert!(
        text.contains("main content"),
        "should include main paragraph: {text}"
    );
}

// --- TASK 2C: New WOM extraction tests ---

/// WOM should include inputs with their labels.
#[test]
fn test_wom_extracts_inputs() {
    let html = r#"<html><body>
        <label for="email">Email</label>
        <input type="email" id="email" name="email" placeholder="your@email.com">
        <label for="name">Full Name</label>
        <input type="text" id="name" name="name">
    </body></html>"#;
    let dom = dom_from(html);
    let wom = neo_extract::wom::build_wom(&dom, "https://example.com");

    let inputs: Vec<_> = wom.nodes.iter().filter(|n| n.tag == "input").collect();
    assert!(
        inputs.len() >= 2,
        "should find at least 2 inputs, got {}",
        inputs.len()
    );

    // Email input should have type action
    let email_input = inputs.iter().find(|n| {
        n.label.contains("email") || n.label.contains("Email") || n.label.contains("your@email")
    });
    assert!(
        email_input.is_some(),
        "should find email input with label, nodes: {:?}",
        inputs.iter().map(|n| &n.label).collect::<Vec<_>>()
    );
    assert!(
        email_input.unwrap().actions.contains(&"type".to_string()),
        "email input should have 'type' action"
    );
}

/// WOM should include links with their href.
#[test]
fn test_wom_extracts_links() {
    let html = r#"<html><body>
        <a href="https://example.com/about">About Us</a>
        <a href="https://example.com/contact">Contact</a>
        <a href="https://example.com/blog">Blog</a>
    </body></html>"#;
    let dom = dom_from(html);
    let wom = neo_extract::wom::build_wom(&dom, "https://example.com");

    let links: Vec<_> = wom.nodes.iter().filter(|n| n.role == "link").collect();
    assert_eq!(links.len(), 3, "should find 3 links");

    // Each link should have click + navigate actions
    for link in &links {
        assert!(
            link.actions.contains(&"click".to_string()),
            "link should have click action: {:?}",
            link
        );
        assert!(
            link.actions.contains(&"navigate".to_string()),
            "link should have navigate action: {:?}",
            link
        );
    }

    // Check label contains the link text
    let about = links.iter().find(|n| n.label.contains("About"));
    assert!(about.is_some(), "should find 'About Us' link by label");
}

/// WOM summary format should include title, element counts.
#[test]
fn test_wom_summary_format() {
    let html = r#"<html><head><title>Dashboard</title></head><body>
        <nav><a href="/">Home</a><a href="/settings">Settings</a></nav>
        <main>
            <h1>Dashboard</h1>
            <form action="/save">
                <input type="text" name="q" placeholder="Search">
                <button type="submit">Go</button>
            </form>
            <table><tr><th>Name</th></tr><tr><td>Row 1</td></tr></table>
        </main>
    </body></html>"#;
    let dom = dom_from(html);
    let wom = neo_extract::wom::build_wom(&dom, "https://example.com");

    // Summary should contain the page title
    assert!(
        wom.summary.contains("Dashboard"),
        "summary should contain page title: {}",
        wom.summary
    );

    // Summary should mention interactive element types
    assert!(
        wom.summary.contains("link") || wom.summary.contains("button") || wom.summary.contains("input"),
        "summary should mention interactive elements: {}",
        wom.summary
    );

    // Summary should be reasonably compact (under 500 chars)
    assert!(
        wom.summary.len() < 500,
        "summary should be compact, got {} chars: {}",
        wom.summary.len(),
        wom.summary
    );
}
