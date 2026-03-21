//! Live DOM bridge — translates interaction operations into JavaScript eval calls
//! on the V8/linkedom runtime.
//!
//! After V8 executes page scripts, the live DOM exists inside linkedom (running in V8).
//! This module bridges AI intents to that live DOM by generating JavaScript snippets
//! and evaluating them through the [`JsRuntime`] trait.

use neo_runtime::{JsRuntime, RuntimeError};
use serde::Deserialize;

/// Bridge between AI intents and the live V8/linkedom DOM.
///
/// Translates high-level operations (click, type, submit) into JavaScript
/// eval calls against the live DOM running inside the V8 runtime.
pub struct LiveDom<'a> {
    runtime: &'a mut dyn JsRuntime,
}

/// Errors from live DOM operations.
#[derive(Debug, thiserror::Error)]
pub enum LiveDomError {
    /// CSS selector matched no elements.
    #[error("element not found: {0}")]
    NotFound(String),

    /// JavaScript evaluation failed.
    #[error("js eval error: {0}")]
    Eval(String),

    /// Timed out waiting for an element to appear.
    #[error("timeout waiting for: {0}")]
    Timeout(String),

    /// Failed to parse JSON response from JS.
    #[error("json parse error: {0}")]
    Parse(String),
}

impl From<RuntimeError> for LiveDomError {
    fn from(e: RuntimeError) -> Self {
        LiveDomError::Eval(e.to_string())
    }
}

/// Successful click response from JS.
#[derive(Debug, Deserialize)]
struct ClickResponse {
    #[allow(dead_code)]
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    text: String,
    #[serde(default)]
    href: String,
    #[serde(default)]
    error: Option<String>,
}

/// Generic ok/error response from JS.
#[derive(Debug, Deserialize)]
struct OkResponse {
    #[allow(dead_code)]
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

/// Submit response with optional action URL.
#[derive(Debug, Deserialize)]
struct SubmitResponse {
    #[allow(dead_code)]
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    action: String,
    #[serde(default)]
    clicked: bool,
    #[serde(default)]
    error: Option<String>,
}

/// Press-key response.
#[derive(Debug, Deserialize)]
struct KeyResponse {
    #[allow(dead_code)]
    #[serde(default)]
    ok: bool,
    #[allow(dead_code)]
    #[serde(default)]
    submitted: bool,
    #[serde(default)]
    error: Option<String>,
}

/// Value/text response.
#[derive(Debug, Deserialize)]
struct ValueResponse {
    #[allow(dead_code)]
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    value: String,
    #[serde(default)]
    error: Option<String>,
}

/// Exists response.
#[derive(Debug, Deserialize)]
struct ExistsResponse {
    #[serde(default)]
    exists: bool,
}

/// Link entry from the page.
#[derive(Debug, Deserialize)]
struct LinkEntry {
    #[serde(default)]
    text: String,
    #[serde(default)]
    href: String,
}

// ─── JS template functions ───────────────────────────────────────────

/// Escape a string for safe embedding in JS string literals (single-quoted).
fn js_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Build the JS snippet for `click`.
pub(crate) fn js_click(selector: &str) -> String {
    let sel = js_escape(selector);
    format!(
        r#"(function(){{
  var el = document.querySelector('{sel}');
  if (!el) return JSON.stringify({{error: 'not found'}});
  el.click();
  return JSON.stringify({{ok: true, text: (el.textContent||'').trim().substring(0,100), href: el.href||''}});
}})()"#
    )
}

/// Build the JS snippet for `type_text`.
pub(crate) fn js_type_text(selector: &str, text: &str) -> String {
    let sel = js_escape(selector);
    let txt = js_escape(text);
    format!(
        r#"(function(){{
  var el = document.querySelector('{sel}');
  if (!el) return JSON.stringify({{error: 'not found'}});
  el.focus();
  el.value = '{txt}';
  el.dispatchEvent(new Event('input', {{bubbles: true}}));
  el.dispatchEvent(new Event('change', {{bubbles: true}}));
  return JSON.stringify({{ok: true}});
}})()"#
    )
}

/// Build the JS snippet for `press_key`.
pub(crate) fn js_press_key(selector: &str, key: &str) -> String {
    let sel = js_escape(selector);
    let k = js_escape(key);
    // For standard keys, code == key. Custom mappings can be added here later.
    let code_esc = js_escape(key);
    format!(
        r#"(function(){{
  var el = document.querySelector('{sel}') || document.activeElement;
  if (!el) return JSON.stringify({{error: 'not found'}});
  var opts = {{key: '{k}', code: '{code_esc}', bubbles: true}};
  el.dispatchEvent(new KeyboardEvent('keydown', opts));
  el.dispatchEvent(new KeyboardEvent('keyup', opts));
  if ('{k}' === 'Enter') {{
    var form = el.closest('form');
    if (form) {{ form.submit(); return JSON.stringify({{ok: true, submitted: true}}); }}
  }}
  return JSON.stringify({{ok: true}});
}})()"#
    )
}

/// Build the JS snippet for `submit`.
pub(crate) fn js_submit(selector: &str) -> String {
    let sel = js_escape(selector);
    format!(
        r#"(function(){{
  var el = document.querySelector('{sel}');
  if (!el) return JSON.stringify({{error: 'not found'}});
  var form = el.tagName === 'FORM' ? el : el.closest('form');
  if (form) {{
    var action = form.action || '';
    form.submit();
    return JSON.stringify({{ok: true, action: action}});
  }}
  el.click();
  return JSON.stringify({{ok: true, clicked: true}});
}})()"#
    )
}

/// Build the JS snippet for `get_value`.
pub(crate) fn js_get_value(selector: &str) -> String {
    let sel = js_escape(selector);
    format!(
        r#"(function(){{
  var el = document.querySelector('{sel}');
  if (!el) return JSON.stringify({{error: 'not found'}});
  var v = ('value' in el) ? el.value : (el.textContent||'');
  return JSON.stringify({{ok: true, value: v}});
}})()"#
    )
}

/// Build the JS snippet for `get_text`.
pub(crate) fn js_get_text(selector: &str) -> String {
    let sel = js_escape(selector);
    format!(
        r#"(function(){{
  var el = document.querySelector('{sel}');
  if (!el) return JSON.stringify({{error: 'not found'}});
  return JSON.stringify({{ok: true, value: (el.textContent||'').trim()}});
}})()"#
    )
}

/// Build the JS snippet for `exists`.
pub(crate) fn js_exists(selector: &str) -> String {
    let sel = js_escape(selector);
    format!(
        r#"(function(){{
  return JSON.stringify({{exists: !!document.querySelector('{sel}')}});
}})()"#
    )
}

/// Build the JS snippet for `page_text`.
pub(crate) fn js_page_text() -> String {
    r#"(function(){
  function walk(el, depth) {
    if (!el || depth > 10) return '';
    var tag = (el.tagName||'').toLowerCase();
    var skip = ['script','style','noscript','svg','path'];
    if (skip.indexOf(tag) >= 0) return '';
    var text = '';
    if (el.nodeType === 3) return (el.textContent||'').trim();
    var children = el.childNodes || [];
    for (var i = 0; i < children.length; i++) {
      text += walk(children[i], depth + 1) + ' ';
    }
    return text.trim();
  }
  return walk(document.body, 0).replace(/\s+/g, ' ').substring(0, 50000);
})()"#
        .to_string()
}

/// Build the JS snippet for `links`.
pub(crate) fn js_links() -> String {
    r#"(function(){
  var links = document.querySelectorAll('a[href]');
  var result = [];
  for (var i = 0; i < links.length; i++) {
    result.push({text: (links[i].textContent||'').trim().substring(0,100), href: links[i].href||''});
  }
  return JSON.stringify(result);
})()"#
        .to_string()
}

/// Build the JS snippet for `current_url`.
pub(crate) fn js_current_url() -> String {
    "(function(){ return document.location ? document.location.href : ''; })()".to_string()
}

/// Build the JS snippet for `title`.
pub(crate) fn js_title() -> String {
    "(function(){ return document.title || ''; })()".to_string()
}

/// Build the JS snippet for `semantic_text`.
pub(crate) fn js_semantic_text() -> String {
    r#"(function(){
  function walk(el, depth) {
    if (!el || depth > 10) return '';
    var tag = (el.tagName||'').toLowerCase();
    var skip = ['script','style','noscript','svg','path'];
    if (skip.indexOf(tag) >= 0) return '';
    var text = '';
    if (el.nodeType === 3) return (el.textContent||'').trim();
    var prefix = '';
    if (tag === 'h1' || tag === 'h2' || tag === 'h3') prefix = '\n## ';
    else if (tag === 'li') prefix = '\n- ';
    else if (tag === 'p' || tag === 'div' || tag === 'section') prefix = '\n';
    var children = el.childNodes || [];
    for (var i = 0; i < children.length; i++) {
      text += walk(children[i], depth + 1) + ' ';
    }
    text = text.trim();
    if (!text) return '';
    if (tag === 'a' && el.href) return '[' + text + '](' + el.href + ')';
    if (tag === 'input' || tag === 'textarea' || tag === 'select') {
      var label = el.getAttribute('aria-label') || el.getAttribute('placeholder') || el.name || '';
      return '[input:' + label + '=' + (el.value||'') + ']';
    }
    if (tag === 'button') return '[button:' + text + ']';
    return prefix + text;
  }
  return walk(document.body, 0).replace(/\n{3,}/g, '\n\n').substring(0, 50000);
})()"#
        .to_string()
}

// ─── LiveDom implementation ──────────────────────────────────────────

impl<'a> LiveDom<'a> {
    /// Create a new LiveDom bridge wrapping a JS runtime.
    pub fn new(runtime: &'a mut dyn JsRuntime) -> Self {
        Self { runtime }
    }

    /// Evaluate JS and parse the JSON result, checking for `error` field.
    fn eval_json<T: serde::de::DeserializeOwned>(&mut self, js: &str) -> Result<T, LiveDomError> {
        let raw = self.runtime.eval(js)?;
        serde_json::from_str(&raw).map_err(|e| {
            LiveDomError::Parse(format!("{e}: raw={}", &raw[..raw.len().min(200)]))
        })
    }

    /// Click an element by CSS selector. Returns the element's text and href.
    pub fn click(&mut self, selector: &str) -> Result<String, LiveDomError> {
        let resp: ClickResponse = self.eval_json(&js_click(selector))?;
        if let Some(err) = resp.error {
            return Err(LiveDomError::NotFound(format!("{selector}: {err}")));
        }
        let mut result = resp.text;
        if !resp.href.is_empty() {
            result = format!("{result} -> {}", resp.href);
        }
        Ok(result)
    }

    /// Type text into an input/textarea. Dispatches `input` and `change` events.
    pub fn type_text(&mut self, selector: &str, text: &str) -> Result<(), LiveDomError> {
        let resp: OkResponse = self.eval_json(&js_type_text(selector, text))?;
        if let Some(err) = resp.error {
            return Err(LiveDomError::NotFound(format!("{selector}: {err}")));
        }
        Ok(())
    }

    /// Press a special key (Enter, Tab, Escape, etc.).
    pub fn press_key(&mut self, selector: &str, key: &str) -> Result<(), LiveDomError> {
        let resp: KeyResponse = self.eval_json(&js_press_key(selector, key))?;
        if let Some(err) = resp.error {
            return Err(LiveDomError::NotFound(format!("{selector}: {err}")));
        }
        Ok(())
    }

    /// Submit a form (finds closest `<form>` and submits, or clicks the element).
    pub fn submit(&mut self, selector: &str) -> Result<String, LiveDomError> {
        let resp: SubmitResponse = self.eval_json(&js_submit(selector))?;
        if let Some(err) = resp.error {
            return Err(LiveDomError::NotFound(format!("{selector}: {err}")));
        }
        if resp.clicked {
            Ok("clicked".to_string())
        } else {
            Ok(resp.action)
        }
    }

    /// Get value of an element (`value` for inputs, `textContent` for others).
    pub fn get_value(&mut self, selector: &str) -> Result<String, LiveDomError> {
        let resp: ValueResponse = self.eval_json(&js_get_value(selector))?;
        if let Some(err) = resp.error {
            return Err(LiveDomError::NotFound(format!("{selector}: {err}")));
        }
        Ok(resp.value)
    }

    /// Get text content of an element (trimmed).
    pub fn get_text(&mut self, selector: &str) -> Result<String, LiveDomError> {
        let resp: ValueResponse = self.eval_json(&js_get_text(selector))?;
        if let Some(err) = resp.error {
            return Err(LiveDomError::NotFound(format!("{selector}: {err}")));
        }
        Ok(resp.value)
    }

    /// Check if an element matching the selector exists.
    pub fn exists(&mut self, selector: &str) -> Result<bool, LiveDomError> {
        let resp: ExistsResponse = self.eval_json(&js_exists(selector))?;
        Ok(resp.exists)
    }

    /// Wait for an element to appear, polling every 100ms up to `timeout_ms`.
    pub fn wait_for(&mut self, selector: &str, timeout_ms: u32) -> Result<bool, LiveDomError> {
        let polls = timeout_ms / 100;
        for _ in 0..polls.max(1) {
            if self.exists(selector)? {
                return Ok(true);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        // Final check.
        if self.exists(selector)? {
            return Ok(true);
        }
        Err(LiveDomError::Timeout(selector.to_string()))
    }

    /// Get the full text content of the page body.
    pub fn page_text(&mut self) -> Result<String, LiveDomError> {
        let raw = self.runtime.eval(&js_page_text())?;
        Ok(raw)
    }

    /// Get all links on the page as `(text, href)` pairs.
    pub fn links(&mut self) -> Result<Vec<(String, String)>, LiveDomError> {
        let raw = self.runtime.eval(&js_links())?;
        let entries: Vec<LinkEntry> = serde_json::from_str(&raw)
            .map_err(|e| LiveDomError::Parse(format!("{e}: raw={}", &raw[..raw.len().min(200)])))?;
        Ok(entries.into_iter().map(|e| (e.text, e.href)).collect())
    }

    /// Get the current page URL.
    pub fn current_url(&mut self) -> Result<String, LiveDomError> {
        let raw = self.runtime.eval(&js_current_url())?;
        Ok(raw)
    }

    /// Get the current page title.
    pub fn title(&mut self) -> Result<String, LiveDomError> {
        let raw = self.runtime.eval(&js_title())?;
        Ok(raw)
    }

    /// Fill multiple form fields at once. Each entry is `(selector, value)`.
    pub fn fill_form(&mut self, fields: &[(&str, &str)]) -> Result<(), LiveDomError> {
        for (selector, value) in fields {
            self.type_text(selector, value)?;
        }
        Ok(())
    }

    /// Execute arbitrary JavaScript and return the result as a string.
    pub fn eval(&mut self, js: &str) -> Result<String, LiveDomError> {
        let raw = self.runtime.eval(js)?;
        Ok(raw)
    }

    /// Extract a compact, AI-friendly text representation of the visible page.
    ///
    /// Headings become `## heading`, links become `[text](href)`,
    /// inputs become `[input:label=value]`, buttons become `[button:text]`.
    pub fn semantic_text(&mut self) -> Result<String, LiveDomError> {
        let raw = self.runtime.eval(&js_semantic_text())?;
        Ok(raw)
    }
}

// ─── Unit tests (JS generation, no V8 needed) ───────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use neo_runtime::mock::MockRuntime;

    #[test]
    fn test_js_click_generation() {
        let js = js_click("button.submit");
        assert!(js.contains("document.querySelector('button.submit')"));
        assert!(js.contains("el.click()"));
    }

    #[test]
    fn test_js_type_text_generation() {
        let js = js_type_text("#email", "user@test.com");
        assert!(js.contains("document.querySelector('#email')"));
        assert!(js.contains("el.value = 'user@test.com'"));
        assert!(js.contains("dispatchEvent"));
    }

    #[test]
    fn test_js_escape_special_chars() {
        let js = js_type_text("#field", "it's a \"test\"\nline2");
        // Single quotes escaped for JS single-quoted string; newlines escaped.
        assert!(js.contains(r"it\'s a "));
        assert!(js.contains(r"\nline2"));
    }

    #[test]
    fn test_js_press_key_enter() {
        let js = js_press_key("#input", "Enter");
        assert!(js.contains("key: 'Enter'"));
        assert!(js.contains("code: 'Enter'"));
        assert!(js.contains("form.submit()"));
    }

    #[test]
    fn test_js_submit_generation() {
        let js = js_submit("form#login");
        assert!(js.contains("document.querySelector('form#login')"));
        assert!(js.contains("form.submit()"));
    }

    #[test]
    fn test_js_exists_generation() {
        let js = js_exists(".modal");
        assert!(js.contains("document.querySelector('.modal')"));
        assert!(js.contains("exists"));
    }

    #[test]
    fn test_js_page_text_generation() {
        let js = js_page_text();
        assert!(js.contains("walk(document.body"));
        assert!(js.contains("substring(0, 50000)"));
    }

    #[test]
    fn test_js_semantic_text_generation() {
        let js = js_semantic_text();
        assert!(js.contains("[button:"));
        assert!(js.contains("[input:"));
        assert!(js.contains("## "));
    }

    #[test]
    fn test_js_links_generation() {
        let js = js_links();
        assert!(js.contains("querySelectorAll('a[href]')"));
    }

    // ─── MockRuntime integration tests ──────────────────────────────

    #[test]
    fn test_live_dom_click_success() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"ok":true,"text":"Login","href":""}"#);
        let mut dom = LiveDom::new(&mut rt);
        let result = dom.click("button.login").unwrap();
        assert_eq!(result, "Login");
    }

    #[test]
    fn test_live_dom_click_with_href() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"ok":true,"text":"Home","href":"https://example.com"}"#);
        let mut dom = LiveDom::new(&mut rt);
        let result = dom.click("a.home").unwrap();
        assert_eq!(result, "Home -> https://example.com");
    }

    #[test]
    fn test_live_dom_click_not_found() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"error":"not found"}"#);
        let mut dom = LiveDom::new(&mut rt);
        let err = dom.click("button.gone").unwrap_err();
        assert!(matches!(err, LiveDomError::NotFound(_)));
    }

    #[test]
    fn test_live_dom_type_text_success() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"ok":true}"#);
        let mut dom = LiveDom::new(&mut rt);
        dom.type_text("#email", "test@example.com").unwrap();
    }

    #[test]
    fn test_live_dom_get_value() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"ok":true,"value":"hello world"}"#);
        let mut dom = LiveDom::new(&mut rt);
        let val = dom.get_value("#name").unwrap();
        assert_eq!(val, "hello world");
    }

    #[test]
    fn test_live_dom_exists_true() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"exists":true}"#);
        let mut dom = LiveDom::new(&mut rt);
        assert!(dom.exists(".modal").unwrap());
    }

    #[test]
    fn test_live_dom_exists_false() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"exists":false}"#);
        let mut dom = LiveDom::new(&mut rt);
        assert!(!dom.exists(".modal").unwrap());
    }

    #[test]
    fn test_live_dom_fill_form() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"ok":true}"#);
        let mut dom = LiveDom::new(&mut rt);
        dom.fill_form(&[("#user", "admin"), ("#pass", "secret")])
            .unwrap();
        // Two type_text calls = two evals.
        assert_eq!(rt.eval_calls.len(), 2);
    }

    #[test]
    fn test_live_dom_page_text() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval("Welcome to the page");
        let mut dom = LiveDom::new(&mut rt);
        let text = dom.page_text().unwrap();
        assert_eq!(text, "Welcome to the page");
    }

    #[test]
    fn test_live_dom_links() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"[{"text":"Home","href":"https://example.com"},{"text":"About","href":"/about"}]"#);
        let mut dom = LiveDom::new(&mut rt);
        let links = dom.links().unwrap();
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].0, "Home");
        assert_eq!(links[1].1, "/about");
    }

    #[test]
    fn test_live_dom_submit_form() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"ok":true,"action":"/login","clicked":false}"#);
        let mut dom = LiveDom::new(&mut rt);
        let result = dom.submit("form#login").unwrap();
        assert_eq!(result, "/login");
    }

    #[test]
    fn test_live_dom_submit_button_click() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"ok":true,"action":"","clicked":true}"#);
        let mut dom = LiveDom::new(&mut rt);
        let result = dom.submit("button.go").unwrap();
        assert_eq!(result, "clicked");
    }

    #[test]
    fn test_live_dom_eval_error() {
        let mut rt = MockRuntime::new();
        rt.eval_error = Some("ReferenceError: x is not defined".to_string());
        let mut dom = LiveDom::new(&mut rt);
        let err = dom.click("button").unwrap_err();
        assert!(matches!(err, LiveDomError::Eval(_)));
    }

    #[test]
    fn test_live_dom_title() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval("My Page Title");
        let mut dom = LiveDom::new(&mut rt);
        let title = dom.title().unwrap();
        assert_eq!(title, "My Page Title");
    }

    #[test]
    fn test_live_dom_current_url() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval("https://example.com/page");
        let mut dom = LiveDom::new(&mut rt);
        let url = dom.current_url().unwrap();
        assert_eq!(url, "https://example.com/page");
    }

    #[test]
    fn test_live_dom_get_text() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"ok":true,"value":"Hello World"}"#);
        let mut dom = LiveDom::new(&mut rt);
        let text = dom.get_text("h1").unwrap();
        assert_eq!(text, "Hello World");
    }

    #[test]
    fn test_live_dom_press_key() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"ok":true,"submitted":false}"#);
        let mut dom = LiveDom::new(&mut rt);
        dom.press_key("#search", "Tab").unwrap();
    }

    #[test]
    fn test_live_dom_semantic_text() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval("## Welcome\n[button:Login]\n[input:email=]");
        let mut dom = LiveDom::new(&mut rt);
        let text = dom.semantic_text().unwrap();
        assert!(text.contains("## Welcome"));
        assert!(text.contains("[button:Login]"));
    }
}
