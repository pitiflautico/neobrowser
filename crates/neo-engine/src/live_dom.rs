//! Live DOM bridge — translates interaction operations into JavaScript eval calls
//! on the V8/linkedom runtime.
//!
//! After V8 executes page scripts, the live DOM exists inside linkedom (running in V8).
//! This module bridges AI intents to that live DOM by generating JavaScript snippets
//! and evaluating them through the [`JsRuntime`] trait.
//!
//! ## Architecture
//!
//! A single JS dispatcher (`window.__neo.exec`) is injected once and handles all
//! commands via typed JSON messages. This prevents injection issues and centralizes
//! all browser-side logic.

use neo_runtime::{JsRuntime, RuntimeError};
use serde::{Deserialize, Serialize};
use std::time::Instant;

// ─── Error types ─────────────────────────────────────────────────────

/// Errors from live DOM operations.
#[derive(Debug, thiserror::Error)]
pub enum LiveDomError {
    /// CSS selector / text / ARIA matched no elements.
    #[error("element not found: {0}")]
    NotFound(String),

    /// Multiple elements matched when exactly one was expected.
    #[error("ambiguous match for '{selector}': {count} candidates — {candidates:?}")]
    AmbiguousMatch {
        selector: String,
        count: usize,
        candidates: Vec<String>,
    },

    /// Element was found but is not interactable (hidden, disabled, etc.).
    #[error("element '{selector}' not interactable: {reason}")]
    NotInteractable { selector: String, reason: String },

    /// Timed out waiting for a condition.
    #[error("timeout waiting for {what} after {elapsed_ms}ms")]
    Timeout { what: String, elapsed_ms: u64 },

    /// Element was found but became detached from the DOM before the action ran.
    #[error("detached node: {0}")]
    DetachedNode(String),

    /// A navigation was aborted (form submit to about:blank, etc.).
    #[error("navigation aborted: {0}")]
    NavigationAborted(String),

    /// JavaScript threw an exception during eval.
    #[error("js exception: {0}")]
    JsException(String),

    /// Attempted to access a cross-origin frame.
    #[error("cross-origin frame: {0}")]
    CrossOriginFrame(String),

    /// Failed to parse JSON response from JS.
    #[error("json parse error: {0}")]
    Parse(String),
}

impl From<RuntimeError> for LiveDomError {
    fn from(e: RuntimeError) -> Self {
        LiveDomError::JsException(e.to_string())
    }
}

// ─── ActionOutcome ───────────────────────────────────────────────────

/// What happened after an action was executed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionOutcome {
    /// Nothing observable changed.
    #[default]
    NoOp,
    /// DOM structure changed (innerHTML length delta).
    DomMutation,
    /// JS state may have changed but no DOM/URL change was detected.
    JsOnlyEffect,
    /// URL hash or pushState changed (SPA navigation).
    SpaRouteChange,
    /// Full page navigation (location.href changed to different origin/path).
    FullNavigation,
    /// A new window/tab context was requested (window.open, target=_blank).
    NewContext,
}

// ─── LiveDomResult ───────────────────────────────────────────────────

/// Rich result wrapper for every LiveDom action.
#[derive(Debug, Clone)]
pub struct LiveDomResult<T> {
    /// The primary return value.
    pub value: T,
    /// What happened after the action.
    pub outcome: ActionOutcome,
    /// Number of DOM mutations detected (innerHTML length delta).
    pub mutations: usize,
    /// Wall-clock time for the operation in milliseconds.
    pub elapsed_ms: u64,
    /// Non-fatal warnings (e.g. "element was partially obscured").
    pub warnings: Vec<String>,
}

impl<T> LiveDomResult<T> {
    fn new(value: T, outcome: ActionOutcome, mutations: usize, elapsed_ms: u64) -> Self {
        Self {
            value,
            outcome,
            mutations,
            elapsed_ms,
            warnings: Vec::new(),
        }
    }

    /// Attach warnings to the result.
    pub fn with_warnings(mut self, warnings: Vec<String>) -> Self {
        self.warnings = warnings;
        self
    }
}

// ─── FrameInfo ───────────────────────────────────────────────────────

/// Information about an iframe in the page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameInfo {
    /// CSS selector that identifies this frame.
    pub selector: String,
    /// The frame's name attribute (empty if unset).
    pub name: String,
    /// The frame's src URL.
    pub src: String,
}

// ─── JS Dispatcher ───────────────────────────────────────────────────

/// The single JS dispatcher function injected into the runtime.
///
/// All LiveDom methods route through `window.__neo.exec(jsonCmd)`.
/// This centralises JS logic, prevents injection issues, and provides
/// consistent element resolution + outcome detection.
const DISPATCHER_JS: &str = r#"
(function() {
  "use strict";
  if (window.__neo && window.__neo._v >= 2) return;
  window.__neo = window.__neo || {};
  window.__neo._v = 2;

  // ── Element resolution ──────────────────────────────────────────
  // Multi-strategy: CSS → text → ARIA, with visibility+interactability filtering.

  function isVisible(el) {
    if (!el) return false;
    // offsetParent is null for hidden elements, except fixed/sticky/body
    if (el.offsetParent === null) {
      var style = el.ownerDocument.defaultView
        ? el.ownerDocument.defaultView.getComputedStyle(el)
        : null;
      if (style) {
        var pos = style.getPropertyValue('position');
        if (pos !== 'fixed' && pos !== 'sticky' && el.tagName !== 'BODY' && el.tagName !== 'HTML') {
          return false;
        }
      } else {
        // linkedom may not have getComputedStyle — assume visible
        return true;
      }
    }
    return true;
  }

  function isEnabled(el) {
    return !el.disabled;
  }

  function describeEl(el) {
    var tag = (el.tagName || '').toLowerCase();
    var id = el.id ? '#' + el.id : '';
    var cls = el.className && typeof el.className === 'string'
      ? '.' + el.className.trim().split(/\s+/).join('.')
      : '';
    var text = (el.textContent || '').trim().substring(0, 40);
    return tag + id + cls + (text ? '("' + text + '")' : '');
  }

  function findByText(text) {
    var lower = text.toLowerCase();
    var all = document.querySelectorAll('a, button, [role="button"], label, span, p, h1, h2, h3, h4, h5, h6, li, td, th, div');
    var results = [];
    for (var i = 0; i < all.length; i++) {
      var t = (all[i].textContent || '').trim();
      if (t.toLowerCase() === lower) results.push(all[i]);
    }
    // If no exact match, try contains
    if (results.length === 0) {
      for (var j = 0; j < all.length; j++) {
        var t2 = (all[j].textContent || '').trim().toLowerCase();
        if (t2.indexOf(lower) >= 0 && t2.length < lower.length * 3) results.push(all[j]);
      }
    }
    return results;
  }

  function findByAria(sel) {
    var results = [];
    // Try role
    var byRole = document.querySelectorAll('[role="' + sel + '"]');
    for (var i = 0; i < byRole.length; i++) results.push(byRole[i]);
    // Try aria-label
    var byLabel = document.querySelectorAll('[aria-label="' + sel + '"]');
    for (var j = 0; j < byLabel.length; j++) {
      if (results.indexOf(byLabel[j]) < 0) results.push(byLabel[j]);
    }
    return results;
  }

  function resolve(selector, opts) {
    opts = opts || {};
    var needsInteractable = opts.interactable || false;
    var candidates = [];

    // Strategy 1: CSS selector
    try {
      var els = document.querySelectorAll(selector);
      for (var i = 0; i < els.length; i++) candidates.push(els[i]);
    } catch(e) {
      // Not a valid CSS selector — that's fine, try text/ARIA
    }

    // Strategy 2: text match (only if CSS found nothing and selector is plain text)
    if (candidates.length === 0 && !/^[#.\[>~+:*]/.test(selector) && selector.indexOf('=') < 0) {
      candidates = findByText(selector);
    }

    // Strategy 3: ARIA
    if (candidates.length === 0) {
      candidates = findByAria(selector);
    }

    // Filter visible
    var visible = [];
    for (var v = 0; v < candidates.length; v++) {
      if (isVisible(candidates[v])) visible.push(candidates[v]);
    }

    // Filter enabled if interactable requested
    var final_list = visible;
    if (needsInteractable) {
      var enabled = [];
      for (var e = 0; e < visible.length; e++) {
        if (isEnabled(visible[e])) enabled.push(visible[e]);
      }
      if (enabled.length > 0) final_list = enabled;
      else if (visible.length > 0) {
        return { error: 'not_interactable', reason: 'all matches are disabled', selector: selector };
      }
    }

    if (final_list.length === 0) {
      return { error: 'not_found', selector: selector };
    }
    if (final_list.length > 5) {
      var descs = [];
      for (var d = 0; d < Math.min(final_list.length, 5); d++) descs.push(describeEl(final_list[d]));
      return { error: 'ambiguous', count: final_list.length, candidates: descs, selector: selector };
    }
    if (final_list.length > 1) {
      // Be lenient: if <=5, pick the first but warn
    }
    return { el: final_list[0], count: final_list.length };
  }

  // ── Outcome detection ───────────────────────────────────────────

  function snapshot() {
    return {
      url: location.href,
      hash: location.hash,
      domLen: document.body ? document.body.innerHTML.length : 0
    };
  }

  function detectOutcome(before, after) {
    if (before.url !== after.url) {
      // Different origin/path → full navigation
      var a = before.url.split('#')[0];
      var b = after.url.split('#')[0];
      if (a !== b) return 'full_navigation';
      return 'spa_route_change';
    }
    if (before.hash !== after.hash) return 'spa_route_change';
    var delta = Math.abs(after.domLen - before.domLen);
    if (delta > 0) return 'dom_mutation';
    return 'no_op';
  }

  // ── Event sequences ─────────────────────────────────────────────

  function fireClick(el) {
    var rect = el.getBoundingClientRect ? el.getBoundingClientRect() : {x:0,y:0,width:0,height:0};
    var cx = rect.x + rect.width / 2;
    var cy = rect.y + rect.height / 2;
    var shared = { bubbles: true, cancelable: true, clientX: cx, clientY: cy, button: 0 };

    el.focus && el.focus();
    try { el.dispatchEvent(new PointerEvent('pointerdown', shared)); } catch(e) {
      el.dispatchEvent(new MouseEvent('pointerdown', shared));
    }
    el.dispatchEvent(new MouseEvent('mousedown', shared));
    try { el.dispatchEvent(new PointerEvent('pointerup', shared)); } catch(e) {
      el.dispatchEvent(new MouseEvent('pointerup', shared));
    }
    el.dispatchEvent(new MouseEvent('mouseup', shared));
    el.dispatchEvent(new MouseEvent('click', shared));
  }

  function fireTypeText(el, text) {
    el.focus && el.focus();
    el.value = '';
    for (var i = 0; i < text.length; i++) {
      var ch = text[i];
      el.dispatchEvent(new KeyboardEvent('keydown', { key: ch, bubbles: true }));
      el.value += ch;
      el.dispatchEvent(new InputEvent('input', { data: ch, inputType: 'insertText', bubbles: true }));
      el.dispatchEvent(new KeyboardEvent('keyup', { key: ch, bubbles: true }));
    }
    el.dispatchEvent(new Event('change', { bubbles: true }));
  }

  function fireSubmit(form) {
    // Blur active element first
    if (document.activeElement && document.activeElement.blur) {
      document.activeElement.blur();
    }
    var ev = new Event('submit', { bubbles: true, cancelable: true });
    var cancelled = !form.dispatchEvent(ev);
    if (!cancelled && form.submit) {
      form.submit();
    }
    return !cancelled;
  }

  function firePressKey(el, key) {
    var opts = { key: key, code: key, bubbles: true, cancelable: true };
    el.dispatchEvent(new KeyboardEvent('keydown', opts));
    el.dispatchEvent(new KeyboardEvent('keyup', opts));
    if (key === 'Enter') {
      var form = el.closest ? el.closest('form') : null;
      if (form) {
        fireSubmit(form);
        return { submitted: true };
      }
    }
    return { submitted: false };
  }

  // ── Dispatcher ──────────────────────────────────────────────────

  window.__neo.exec = function(cmdJson) {
    var cmd;
    try { cmd = JSON.parse(cmdJson); } catch(e) {
      return JSON.stringify({ error: 'parse', message: 'invalid JSON: ' + e.message });
    }

    var action = cmd.action;
    var selector = cmd.selector || '';
    var value = cmd.value || '';
    var key = cmd.key || '';
    var frame_sel = cmd.frame || '';

    var before = snapshot();

    try {
      switch(action) {
        case 'click': {
          var r = resolve(selector, { interactable: true });
          if (r.error) return JSON.stringify(r);
          fireClick(r.el);
          var after = snapshot();
          var mutations = Math.abs(after.domLen - before.domLen);
          return JSON.stringify({
            ok: true,
            text: (r.el.textContent || '').trim().substring(0, 100),
            href: r.el.href || '',
            outcome: detectOutcome(before, after),
            mutations: mutations,
            count: r.count
          });
        }

        case 'type_text': {
          var r = resolve(selector, { interactable: true });
          if (r.error) return JSON.stringify(r);
          fireTypeText(r.el, value);
          var after = snapshot();
          return JSON.stringify({
            ok: true,
            outcome: detectOutcome(before, after),
            mutations: Math.abs(after.domLen - before.domLen),
            count: r.count
          });
        }

        case 'press_key': {
          var r = resolve(selector);
          if (r.error) {
            // Fallback to activeElement
            var active = document.activeElement;
            if (!active) return JSON.stringify(r);
            var info = firePressKey(active, key);
            var after = snapshot();
            return JSON.stringify({
              ok: true,
              submitted: info.submitted,
              outcome: detectOutcome(before, after),
              mutations: Math.abs(after.domLen - before.domLen)
            });
          }
          var info = firePressKey(r.el, key);
          var after = snapshot();
          return JSON.stringify({
            ok: true,
            submitted: info.submitted,
            outcome: detectOutcome(before, after),
            mutations: Math.abs(after.domLen - before.domLen),
            count: r.count
          });
        }

        case 'submit': {
          var r = resolve(selector);
          if (r.error) return JSON.stringify(r);
          var el = r.el;
          var form = el.tagName === 'FORM' ? el : (el.closest ? el.closest('form') : null);
          if (form) {
            var action_url = form.action || '';
            var submitted = fireSubmit(form);
            var after = snapshot();
            return JSON.stringify({
              ok: true,
              action: action_url,
              clicked: false,
              submitted: submitted,
              outcome: detectOutcome(before, after),
              mutations: Math.abs(after.domLen - before.domLen)
            });
          }
          // No form — click the element as fallback
          fireClick(el);
          var after = snapshot();
          return JSON.stringify({
            ok: true,
            action: '',
            clicked: true,
            submitted: false,
            outcome: detectOutcome(before, after),
            mutations: Math.abs(after.domLen - before.domLen)
          });
        }

        case 'get_value': {
          var r = resolve(selector);
          if (r.error) return JSON.stringify(r);
          var v = ('value' in r.el) ? r.el.value : (r.el.textContent || '');
          return JSON.stringify({ ok: true, value: v });
        }

        case 'get_text': {
          var r = resolve(selector);
          if (r.error) return JSON.stringify(r);
          return JSON.stringify({ ok: true, value: (r.el.textContent || '').trim() });
        }

        case 'exists': {
          var r = resolve(selector);
          return JSON.stringify({ exists: !r.error, count: r.count || 0 });
        }

        case 'page_text': {
          function walk(el, depth) {
            if (!el || depth > 10) return '';
            var tag = (el.tagName || '').toLowerCase();
            var skip = ['script','style','noscript','svg','path'];
            if (skip.indexOf(tag) >= 0) return '';
            if (el.nodeType === 3) return (el.textContent || '').trim();
            var text = '';
            var children = el.childNodes || [];
            for (var i = 0; i < children.length; i++) {
              text += walk(children[i], depth + 1) + ' ';
            }
            return text.trim();
          }
          return walk(document.body, 0).replace(/\s+/g, ' ').substring(0, 50000);
        }

        case 'semantic_text': {
          function semWalk(el, depth) {
            if (!el || depth > 10) return '';
            var tag = (el.tagName || '').toLowerCase();
            var skip = ['script','style','noscript','svg','path'];
            if (skip.indexOf(tag) >= 0) return '';
            if (el.nodeType === 3) return (el.textContent || '').trim();
            var prefix = '';
            if (tag === 'h1' || tag === 'h2' || tag === 'h3') prefix = '\n## ';
            else if (tag === 'li') prefix = '\n- ';
            else if (tag === 'p' || tag === 'div' || tag === 'section') prefix = '\n';
            var text = '';
            var children = el.childNodes || [];
            for (var i = 0; i < children.length; i++) {
              text += semWalk(children[i], depth + 1) + ' ';
            }
            text = text.trim();
            if (!text) return '';
            if (tag === 'a' && el.href) return '[' + text + '](' + el.href + ')';
            if (tag === 'input' || tag === 'textarea' || tag === 'select') {
              var label = el.getAttribute('aria-label') || el.getAttribute('placeholder') || el.name || '';
              return '[input:' + label + '=' + (el.value || '') + ']';
            }
            if (tag === 'button') return '[button:' + text + ']';
            return prefix + text;
          }
          return semWalk(document.body, 0).replace(/\n{3,}/g, '\n\n').substring(0, 50000);
        }

        case 'links': {
          var links = document.querySelectorAll('a[href]');
          var result = [];
          for (var i = 0; i < links.length; i++) {
            result.push({ text: (links[i].textContent || '').trim().substring(0, 100), href: links[i].href || '' });
          }
          return JSON.stringify(result);
        }

        case 'current_url': {
          return document.location ? document.location.href : '';
        }

        case 'title': {
          return document.title || '';
        }

        case 'list_frames': {
          var iframes = document.querySelectorAll('iframe');
          var result = [];
          for (var i = 0; i < iframes.length; i++) {
            var f = iframes[i];
            var sel_str = f.id ? '#' + f.id : (f.name ? 'iframe[name="' + f.name + '"]' : 'iframe:nth-of-type(' + (i+1) + ')');
            result.push({ selector: sel_str, name: f.name || '', src: f.src || '' });
          }
          return JSON.stringify(result);
        }

        case 'dom_length': {
          return JSON.stringify({ length: document.body ? document.body.innerHTML.length : 0 });
        }

        case 'wait_text': {
          var r = resolve(selector);
          if (r.error) return JSON.stringify({ found: false });
          var t = (r.el.textContent || '').trim();
          return JSON.stringify({ found: t.indexOf(value) >= 0, text: t.substring(0, 200) });
        }

        default:
          return JSON.stringify({ error: 'unknown_action', message: 'unknown action: ' + action });
      }
    } catch(ex) {
      return JSON.stringify({ error: 'exception', message: String(ex.message || ex) });
    }
  };
})();
"#;

// ─── JS response types ──────────────────────────────────────────────

/// Generic dispatcher response — all responses share these fields.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DispatchResponse {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    outcome: Option<String>,
    #[serde(default)]
    mutations: Option<usize>,
    #[serde(default)]
    count: Option<usize>,
    #[serde(default)]
    candidates: Option<Vec<String>>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    selector: Option<String>,
    // Action-specific fields
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    href: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    clicked: Option<bool>,
    #[serde(default)]
    submitted: Option<bool>,
    #[serde(default)]
    exists: Option<bool>,
}

/// Link entry from the page.
#[derive(Debug, Deserialize)]
struct LinkEntry {
    #[serde(default)]
    text: String,
    #[serde(default)]
    href: String,
}

/// Exists response.
#[derive(Debug, Deserialize)]
struct ExistsResponse {
    #[serde(default)]
    exists: bool,
}

/// DOM length response.
#[derive(Debug, Deserialize)]
struct DomLengthResponse {
    #[serde(default)]
    length: usize,
}

/// Wait-for-text response.
#[derive(Debug, Deserialize)]
struct WaitTextResponse {
    #[serde(default)]
    found: bool,
}

// ─── Helpers ─────────────────────────────────────────────────────────

/// Escape a string for safe embedding in JS JSON strings.
fn js_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn parse_outcome(s: &str) -> ActionOutcome {
    match s {
        "dom_mutation" => ActionOutcome::DomMutation,
        "spa_route_change" => ActionOutcome::SpaRouteChange,
        "full_navigation" => ActionOutcome::FullNavigation,
        "new_context" => ActionOutcome::NewContext,
        "js_only_effect" => ActionOutcome::JsOnlyEffect,
        _ => ActionOutcome::NoOp,
    }
}

// ─── LiveDom implementation ──────────────────────────────────────────

/// Bridge between AI intents and the live V8/linkedom DOM.
///
/// Translates high-level operations (click, type, submit) into JavaScript
/// eval calls against the live DOM running inside the V8 runtime.
///
/// All commands route through a single JS dispatcher (`window.__neo.exec`)
/// which provides multi-strategy element resolution, real event sequences,
/// and action outcome detection.
pub struct LiveDom<'a> {
    runtime: &'a mut dyn JsRuntime,
    /// Whether the dispatcher has been injected.
    dispatcher_ready: bool,
    /// Current frame context (empty = top frame).
    frame_context: String,
}

impl<'a> LiveDom<'a> {
    /// Create a new LiveDom bridge wrapping a JS runtime.
    pub fn new(runtime: &'a mut dyn JsRuntime) -> Self {
        Self {
            runtime,
            dispatcher_ready: false,
            frame_context: String::new(),
        }
    }

    /// Ensure the dispatcher JS is injected. Idempotent.
    fn ensure_dispatcher(&mut self) -> Result<(), LiveDomError> {
        if !self.dispatcher_ready {
            self.runtime.execute(DISPATCHER_JS)?;
            self.dispatcher_ready = true;
        }
        Ok(())
    }

    /// Build a JSON command string for the dispatcher.
    fn build_cmd(action: &str, selector: &str, value: &str, key: &str) -> String {
        format!(
            r#"{{"action":"{}","selector":"{}","value":"{}","key":"{}"}}"#,
            js_escape(action),
            js_escape(selector),
            js_escape(value),
            js_escape(key),
        )
    }

    /// Execute a dispatcher command and parse the response.
    fn dispatch(&mut self, action: &str, selector: &str, value: &str, key: &str) -> Result<DispatchResponse, LiveDomError> {
        self.ensure_dispatcher()?;
        let cmd = Self::build_cmd(action, selector, value, key);
        let js = format!("window.__neo.exec('{}')", cmd.replace('\'', "\\'"));
        let raw = self.runtime.eval(&js)?;
        serde_json::from_str(&raw).map_err(|e| {
            LiveDomError::Parse(format!("{e}: raw={}", &raw[..raw.len().min(200)]))
        })
    }

    /// Execute a dispatcher command for actions that return raw strings (page_text, etc.).
    fn dispatch_raw(&mut self, action: &str, selector: &str, value: &str, key: &str) -> Result<String, LiveDomError> {
        self.ensure_dispatcher()?;
        let cmd = Self::build_cmd(action, selector, value, key);
        let js = format!("window.__neo.exec('{}')", cmd.replace('\'', "\\'"));
        let raw = self.runtime.eval(&js)?;
        Ok(raw)
    }

    /// Convert a dispatch response error into a LiveDomError.
    fn check_error(resp: &DispatchResponse) -> Option<LiveDomError> {
        let err = resp.error.as_deref()?;
        match err {
            "not_found" => Some(LiveDomError::NotFound(
                resp.selector.clone().unwrap_or_default(),
            )),
            "ambiguous" => Some(LiveDomError::AmbiguousMatch {
                selector: resp.selector.clone().unwrap_or_default(),
                count: resp.count.unwrap_or(0),
                candidates: resp.candidates.clone().unwrap_or_default(),
            }),
            "not_interactable" => Some(LiveDomError::NotInteractable {
                selector: resp.selector.clone().unwrap_or_default(),
                reason: resp.reason.clone().unwrap_or_default(),
            }),
            "exception" => Some(LiveDomError::JsException(
                resp.message.clone().unwrap_or_default(),
            )),
            _ => Some(LiveDomError::JsException(
                resp.message.clone().unwrap_or_else(|| err.to_string()),
            )),
        }
    }

    /// Extract outcome and mutations from a dispatch response.
    fn extract_outcome(resp: &DispatchResponse) -> (ActionOutcome, usize) {
        let outcome = resp.outcome.as_deref().map(parse_outcome).unwrap_or_default();
        let mutations = resp.mutations.unwrap_or(0);
        (outcome, mutations)
    }

    // ─── Public API ──────────────────────────────────────────────────

    /// Click an element. Returns the element's text and href.
    pub fn click(&mut self, selector: &str) -> Result<LiveDomResult<String>, LiveDomError> {
        let start = Instant::now();
        let resp = self.dispatch("click", selector, "", "")?;
        if let Some(e) = Self::check_error(&resp) {
            return Err(e);
        }
        let (outcome, mutations) = Self::extract_outcome(&resp);
        let text = resp.text.unwrap_or_default();
        let href = resp.href.unwrap_or_default();
        let mut result = text.clone();
        if !href.is_empty() {
            result = format!("{text} -> {href}");
        }
        let elapsed = start.elapsed().as_millis() as u64;
        let mut r = LiveDomResult::new(result, outcome, mutations, elapsed);
        if resp.count.unwrap_or(1) > 1 {
            r.warnings.push(format!(
                "matched {} elements, used first",
                resp.count.unwrap_or(1)
            ));
        }
        Ok(r)
    }

    /// Type text into an input/textarea. Fires per-character keydown/input/keyup + change.
    pub fn type_text(&mut self, selector: &str, text: &str) -> Result<LiveDomResult<()>, LiveDomError> {
        let start = Instant::now();
        let resp = self.dispatch("type_text", selector, text, "")?;
        if let Some(e) = Self::check_error(&resp) {
            return Err(e);
        }
        let (outcome, mutations) = Self::extract_outcome(&resp);
        let elapsed = start.elapsed().as_millis() as u64;
        Ok(LiveDomResult::new((), outcome, mutations, elapsed))
    }

    /// Press a special key (Enter, Tab, Escape, etc.).
    pub fn press_key(&mut self, selector: &str, key: &str) -> Result<LiveDomResult<()>, LiveDomError> {
        let start = Instant::now();
        let resp = self.dispatch("press_key", selector, "", key)?;
        if let Some(e) = Self::check_error(&resp) {
            return Err(e);
        }
        let (outcome, mutations) = Self::extract_outcome(&resp);
        let elapsed = start.elapsed().as_millis() as u64;
        Ok(LiveDomResult::new((), outcome, mutations, elapsed))
    }

    /// Submit a form (finds closest `<form>` and submits, or clicks the element).
    pub fn submit(&mut self, selector: &str) -> Result<LiveDomResult<String>, LiveDomError> {
        let start = Instant::now();
        let resp = self.dispatch("submit", selector, "", "")?;
        if let Some(e) = Self::check_error(&resp) {
            return Err(e);
        }
        let (outcome, mutations) = Self::extract_outcome(&resp);
        let elapsed = start.elapsed().as_millis() as u64;
        let value = if resp.clicked.unwrap_or(false) {
            "clicked".to_string()
        } else {
            resp.action.unwrap_or_default()
        };
        Ok(LiveDomResult::new(value, outcome, mutations, elapsed))
    }

    /// Get value of an element (`value` for inputs, `textContent` for others).
    pub fn get_value(&mut self, selector: &str) -> Result<LiveDomResult<String>, LiveDomError> {
        let start = Instant::now();
        let resp = self.dispatch("get_value", selector, "", "")?;
        if let Some(e) = Self::check_error(&resp) {
            return Err(e);
        }
        let elapsed = start.elapsed().as_millis() as u64;
        Ok(LiveDomResult::new(
            resp.value.unwrap_or_default(),
            ActionOutcome::NoOp,
            0,
            elapsed,
        ))
    }

    /// Get text content of an element (trimmed).
    pub fn get_text(&mut self, selector: &str) -> Result<LiveDomResult<String>, LiveDomError> {
        let start = Instant::now();
        let resp = self.dispatch("get_text", selector, "", "")?;
        if let Some(e) = Self::check_error(&resp) {
            return Err(e);
        }
        let elapsed = start.elapsed().as_millis() as u64;
        Ok(LiveDomResult::new(
            resp.value.unwrap_or_default(),
            ActionOutcome::NoOp,
            0,
            elapsed,
        ))
    }

    /// Check if an element matching the selector exists.
    pub fn exists(&mut self, selector: &str) -> Result<bool, LiveDomError> {
        self.ensure_dispatcher()?;
        let cmd = Self::build_cmd("exists", selector, "", "");
        let js = format!("window.__neo.exec('{}')", cmd.replace('\'', "\\'"));
        let raw = self.runtime.eval(&js)?;
        let resp: ExistsResponse = serde_json::from_str(&raw)
            .map_err(|e| LiveDomError::Parse(format!("{e}: raw={}", &raw[..raw.len().min(200)])))?;
        Ok(resp.exists)
    }

    /// Wait for an element to appear, polling every 100ms up to `timeout_ms`.
    pub fn wait_for_selector(&mut self, selector: &str, timeout_ms: u32) -> Result<LiveDomResult<bool>, LiveDomError> {
        let start = Instant::now();
        let polls = timeout_ms / 100;
        for _ in 0..polls.max(1) {
            if self.exists(selector)? {
                let elapsed = start.elapsed().as_millis() as u64;
                return Ok(LiveDomResult::new(true, ActionOutcome::NoOp, 0, elapsed));
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        // Final check
        if self.exists(selector)? {
            let elapsed = start.elapsed().as_millis() as u64;
            return Ok(LiveDomResult::new(true, ActionOutcome::NoOp, 0, elapsed));
        }
        let elapsed = start.elapsed().as_millis() as u64;
        Err(LiveDomError::Timeout {
            what: format!("selector '{selector}'"),
            elapsed_ms: elapsed,
        })
    }

    /// Wait for an element to contain specific text, polling every 100ms.
    pub fn wait_for_text(&mut self, selector: &str, text: &str, timeout_ms: u32) -> Result<LiveDomResult<bool>, LiveDomError> {
        let start = Instant::now();
        let polls = timeout_ms / 100;
        for _ in 0..polls.max(1) {
            self.ensure_dispatcher()?;
            let cmd = Self::build_cmd("wait_text", selector, text, "");
            let js = format!("window.__neo.exec('{}')", cmd.replace('\'', "\\'"));
            let raw = self.runtime.eval(&js)?;
            if let Ok(wt) = serde_json::from_str::<WaitTextResponse>(&raw) {
                if wt.found {
                    let elapsed = start.elapsed().as_millis() as u64;
                    return Ok(LiveDomResult::new(true, ActionOutcome::NoOp, 0, elapsed));
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let elapsed = start.elapsed().as_millis() as u64;
        Err(LiveDomError::Timeout {
            what: format!("text '{text}' in '{selector}'"),
            elapsed_ms: elapsed,
        })
    }

    /// Wait for the DOM to stop changing. Polls DOM length every 100ms, requires
    /// `quiet_ms` consecutive milliseconds with no change.
    pub fn wait_for_stable(&mut self, timeout_ms: u32, quiet_ms: u32) -> Result<LiveDomResult<()>, LiveDomError> {
        let start = Instant::now();
        let mut last_len = self.dom_length()?;
        let mut stable_since = Instant::now();

        loop {
            let elapsed = start.elapsed().as_millis() as u64;
            if elapsed > timeout_ms as u64 {
                return Err(LiveDomError::Timeout {
                    what: format!("DOM stable for {quiet_ms}ms"),
                    elapsed_ms: elapsed,
                });
            }

            std::thread::sleep(std::time::Duration::from_millis(100));

            let current_len = self.dom_length()?;
            if current_len != last_len {
                last_len = current_len;
                stable_since = Instant::now();
            }

            if stable_since.elapsed().as_millis() as u32 >= quiet_ms {
                let total_elapsed = start.elapsed().as_millis() as u64;
                return Ok(LiveDomResult::new((), ActionOutcome::NoOp, 0, total_elapsed));
            }
        }
    }

    /// Get current DOM body innerHTML length (for stability detection).
    fn dom_length(&mut self) -> Result<usize, LiveDomError> {
        self.ensure_dispatcher()?;
        let cmd = Self::build_cmd("dom_length", "", "", "");
        let js = format!("window.__neo.exec('{}')", cmd.replace('\'', "\\'"));
        let raw = self.runtime.eval(&js)?;
        let resp: DomLengthResponse = serde_json::from_str(&raw)
            .map_err(|e| LiveDomError::Parse(format!("{e}: raw={}", &raw[..raw.len().min(200)])))?;
        Ok(resp.length)
    }

    /// Get the full text content of the page body.
    pub fn page_text(&mut self) -> Result<String, LiveDomError> {
        self.dispatch_raw("page_text", "", "", "")
    }

    /// Get all links on the page as `(text, href)` pairs.
    pub fn links(&mut self) -> Result<Vec<(String, String)>, LiveDomError> {
        let raw = self.dispatch_raw("links", "", "", "")?;
        let entries: Vec<LinkEntry> = serde_json::from_str(&raw)
            .map_err(|e| LiveDomError::Parse(format!("{e}: raw={}", &raw[..raw.len().min(200)])))?;
        Ok(entries.into_iter().map(|e| (e.text, e.href)).collect())
    }

    /// Get the current page URL.
    pub fn current_url(&mut self) -> Result<String, LiveDomError> {
        self.dispatch_raw("current_url", "", "", "")
    }

    /// Get the current page title.
    pub fn title(&mut self) -> Result<String, LiveDomError> {
        self.dispatch_raw("title", "", "", "")
    }

    /// Fill multiple form fields at once. Each entry is `(selector, value)`.
    pub fn fill_form(&mut self, fields: &[(&str, &str)]) -> Result<LiveDomResult<()>, LiveDomError> {
        let start = Instant::now();
        let mut total_mutations = 0usize;
        let mut last_outcome = ActionOutcome::NoOp;
        for (selector, value) in fields {
            let r = self.type_text(selector, value)?;
            total_mutations = total_mutations.saturating_add(r.mutations);
            if r.outcome != ActionOutcome::NoOp {
                last_outcome = r.outcome;
            }
        }
        let elapsed = start.elapsed().as_millis() as u64;
        Ok(LiveDomResult::new((), last_outcome, total_mutations, elapsed))
    }

    /// Execute arbitrary JavaScript and return the result as a string.
    pub fn eval(&mut self, js: &str) -> Result<String, LiveDomError> {
        let raw = self.runtime.eval(js)?;
        Ok(raw)
    }

    /// Extract a compact, AI-friendly text representation of the visible page.
    pub fn semantic_text(&mut self) -> Result<String, LiveDomError> {
        self.dispatch_raw("semantic_text", "", "", "")
    }

    // ─── Frame support ───────────────────────────────────────────────

    /// Switch eval context to a specific iframe.
    ///
    /// Note: In linkedom (server-side), iframes don't have separate documents.
    /// This stores the frame selector for commands that need it. In a real
    /// browser CDP integration, this would switch the execution context.
    pub fn frame(&mut self, selector: &str) -> Result<(), LiveDomError> {
        // Verify the frame exists
        if !self.exists(selector)? {
            return Err(LiveDomError::NotFound(format!("frame '{selector}'")));
        }
        self.frame_context = selector.to_string();
        Ok(())
    }

    /// Switch back to the top-level document context.
    pub fn default_frame(&mut self) {
        self.frame_context.clear();
    }

    /// List all iframes in the current document.
    pub fn list_frames(&mut self) -> Result<Vec<FrameInfo>, LiveDomError> {
        let raw = self.dispatch_raw("list_frames", "", "", "")?;
        let frames: Vec<FrameInfo> = serde_json::from_str(&raw)
            .map_err(|e| LiveDomError::Parse(format!("{e}: raw={}", &raw[..raw.len().min(200)])))?;
        Ok(frames)
    }

    // ─── Legacy compatibility ────────────────────────────────────────

    /// Wait for an element to appear (legacy signature, delegates to wait_for_selector).
    pub fn wait_for(&mut self, selector: &str, timeout_ms: u32) -> Result<bool, LiveDomError> {
        match self.wait_for_selector(selector, timeout_ms) {
            Ok(r) => Ok(r.value),
            Err(e) => Err(e),
        }
    }
}

// ─── Unit tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use neo_runtime::mock::MockRuntime;

    /// Helper: create a LiveDom with a mock that returns a fixed value.
    /// The first eval call is the dispatcher injection (execute), subsequent are eval.
    fn mock_dom(default_response: &str) -> (MockRuntime,) {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(default_response);
        (rt,)
    }

    // ─── Dispatcher injection tests ──────────────────────────────────

    #[test]
    fn test_dispatcher_injected_once() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"exists":true}"#);
        let mut dom = LiveDom::new(&mut rt);
        dom.exists(".a").unwrap();
        dom.exists(".b").unwrap();
        // First call is execute (dispatcher), then 2 evals
        assert_eq!(rt.eval_calls.len(), 3); // 1 execute + 2 eval
    }

    // ─── Click tests ────────────────────────────────────────────────

    #[test]
    fn test_click_success() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(
            r#"{"ok":true,"text":"Login","href":"","outcome":"dom_mutation","mutations":42,"count":1}"#,
        );
        let mut dom = LiveDom::new(&mut rt);
        let result = dom.click("button.login").unwrap();
        assert_eq!(result.value, "Login");
        assert_eq!(result.outcome, ActionOutcome::DomMutation);
        assert_eq!(result.mutations, 42);
    }

    #[test]
    fn test_click_with_href() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(
            r#"{"ok":true,"text":"Home","href":"https://example.com","outcome":"no_op","mutations":0}"#,
        );
        let mut dom = LiveDom::new(&mut rt);
        let result = dom.click("a.home").unwrap();
        assert_eq!(result.value, "Home -> https://example.com");
    }

    #[test]
    fn test_click_not_found() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"error":"not_found","selector":"button.gone"}"#);
        let mut dom = LiveDom::new(&mut rt);
        let err = dom.click("button.gone").unwrap_err();
        assert!(matches!(err, LiveDomError::NotFound(_)));
    }

    #[test]
    fn test_click_ambiguous() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(
            r#"{"error":"ambiguous","selector":".btn","count":10,"candidates":["button.a","button.b"]}"#,
        );
        let mut dom = LiveDom::new(&mut rt);
        let err = dom.click(".btn").unwrap_err();
        match err {
            LiveDomError::AmbiguousMatch { count, candidates, .. } => {
                assert_eq!(count, 10);
                assert_eq!(candidates.len(), 2);
            }
            other => panic!("expected AmbiguousMatch, got {other:?}"),
        }
    }

    #[test]
    fn test_click_not_interactable() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(
            r##"{"error":"not_interactable","selector":"#btn","reason":"all matches are disabled"}"##,
        );
        let mut dom = LiveDom::new(&mut rt);
        let err = dom.click("#btn").unwrap_err();
        assert!(matches!(err, LiveDomError::NotInteractable { .. }));
    }

    // ─── Type text tests ─────────────────────────────────────────────

    #[test]
    fn test_type_text_success() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"ok":true,"outcome":"dom_mutation","mutations":10}"#);
        let mut dom = LiveDom::new(&mut rt);
        let result = dom.type_text("#email", "test@example.com").unwrap();
        assert_eq!(result.outcome, ActionOutcome::DomMutation);
    }

    // ─── Get value/text tests ────────────────────────────────────────

    #[test]
    fn test_get_value() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"ok":true,"value":"hello world"}"#);
        let mut dom = LiveDom::new(&mut rt);
        let val = dom.get_value("#name").unwrap();
        assert_eq!(val.value, "hello world");
    }

    #[test]
    fn test_get_text() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"ok":true,"value":"Hello World"}"#);
        let mut dom = LiveDom::new(&mut rt);
        let text = dom.get_text("h1").unwrap();
        assert_eq!(text.value, "Hello World");
    }

    // ─── Exists tests ────────────────────────────────────────────────

    #[test]
    fn test_exists_true() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"exists":true}"#);
        let mut dom = LiveDom::new(&mut rt);
        assert!(dom.exists(".modal").unwrap());
    }

    #[test]
    fn test_exists_false() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"exists":false}"#);
        let mut dom = LiveDom::new(&mut rt);
        assert!(!dom.exists(".modal").unwrap());
    }

    // ─── Submit tests ────────────────────────────────────────────────

    #[test]
    fn test_submit_form() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(
            r#"{"ok":true,"action":"/login","clicked":false,"submitted":true,"outcome":"full_navigation","mutations":100}"#,
        );
        let mut dom = LiveDom::new(&mut rt);
        let result = dom.submit("form#login").unwrap();
        assert_eq!(result.value, "/login");
        assert_eq!(result.outcome, ActionOutcome::FullNavigation);
    }

    #[test]
    fn test_submit_button_click() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(
            r#"{"ok":true,"action":"","clicked":true,"submitted":false,"outcome":"dom_mutation","mutations":5}"#,
        );
        let mut dom = LiveDom::new(&mut rt);
        let result = dom.submit("button.go").unwrap();
        assert_eq!(result.value, "clicked");
    }

    // ─── Fill form tests ─────────────────────────────────────────────

    #[test]
    fn test_fill_form() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"ok":true,"outcome":"dom_mutation","mutations":5}"#);
        let mut dom = LiveDom::new(&mut rt);
        let result = dom
            .fill_form(&[("#user", "admin"), ("#pass", "secret")])
            .unwrap();
        // 1 execute (dispatcher) + 2 evals (type_text x2)
        assert_eq!(rt.eval_calls.len(), 3);
        assert_eq!(result.mutations, 10); // 5 + 5
    }

    // ─── Page text / links / title / url tests ───────────────────────

    #[test]
    fn test_page_text() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval("Welcome to the page");
        let mut dom = LiveDom::new(&mut rt);
        let text = dom.page_text().unwrap();
        assert_eq!(text, "Welcome to the page");
    }

    #[test]
    fn test_links() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(
            r#"[{"text":"Home","href":"https://example.com"},{"text":"About","href":"/about"}]"#,
        );
        let mut dom = LiveDom::new(&mut rt);
        let links = dom.links().unwrap();
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].0, "Home");
        assert_eq!(links[1].1, "/about");
    }

    #[test]
    fn test_title() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval("My Page Title");
        let mut dom = LiveDom::new(&mut rt);
        let title = dom.title().unwrap();
        assert_eq!(title, "My Page Title");
    }

    #[test]
    fn test_current_url() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval("https://example.com/page");
        let mut dom = LiveDom::new(&mut rt);
        let url = dom.current_url().unwrap();
        assert_eq!(url, "https://example.com/page");
    }

    // ─── Semantic text tests ─────────────────────────────────────────

    #[test]
    fn test_semantic_text() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval("## Welcome\n[button:Login]\n[input:email=]");
        let mut dom = LiveDom::new(&mut rt);
        let text = dom.semantic_text().unwrap();
        assert!(text.contains("## Welcome"));
        assert!(text.contains("[button:Login]"));
    }

    // ─── Press key tests ─────────────────────────────────────────────

    #[test]
    fn test_press_key() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(
            r#"{"ok":true,"submitted":false,"outcome":"no_op","mutations":0}"#,
        );
        let mut dom = LiveDom::new(&mut rt);
        let result = dom.press_key("#search", "Tab").unwrap();
        assert_eq!(result.outcome, ActionOutcome::NoOp);
    }

    // ─── Eval error tests ────────────────────────────────────────────

    #[test]
    fn test_eval_error() {
        let mut rt = MockRuntime::new();
        rt.eval_error = Some("ReferenceError: x is not defined".to_string());
        let mut dom = LiveDom::new(&mut rt);
        let err = dom.click("button").unwrap_err();
        assert!(matches!(err, LiveDomError::JsException(_)));
    }

    // ─── ActionOutcome tests ─────────────────────────────────────────

    #[test]
    fn test_outcome_dom_mutation() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(
            r#"{"ok":true,"text":"X","href":"","outcome":"dom_mutation","mutations":150}"#,
        );
        let mut dom = LiveDom::new(&mut rt);
        let result = dom.click(".close").unwrap();
        assert_eq!(result.outcome, ActionOutcome::DomMutation);
        assert_eq!(result.mutations, 150);
    }

    #[test]
    fn test_outcome_spa_route() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(
            r#"{"ok":true,"text":"About","href":"/about","outcome":"spa_route_change","mutations":500}"#,
        );
        let mut dom = LiveDom::new(&mut rt);
        let result = dom.click("a.about").unwrap();
        assert_eq!(result.outcome, ActionOutcome::SpaRouteChange);
    }

    #[test]
    fn test_outcome_full_navigation() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(
            r#"{"ok":true,"text":"Go","href":"","outcome":"full_navigation","mutations":0}"#,
        );
        let mut dom = LiveDom::new(&mut rt);
        let result = dom.click("a.external").unwrap();
        assert_eq!(result.outcome, ActionOutcome::FullNavigation);
    }

    // ─── Error type tests ────────────────────────────────────────────

    #[test]
    fn test_error_display() {
        let err = LiveDomError::Timeout {
            what: "selector '.x'".to_string(),
            elapsed_ms: 5000,
        };
        assert!(err.to_string().contains("5000ms"));

        let err2 = LiveDomError::AmbiguousMatch {
            selector: ".btn".to_string(),
            count: 3,
            candidates: vec!["a".to_string(), "b".to_string()],
        };
        assert!(err2.to_string().contains("3 candidates"));
    }

    // ─── Frame support tests ─────────────────────────────────────────

    #[test]
    fn test_list_frames() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(
            r##"[{"selector":"#content","name":"content","src":"https://example.com/frame"}]"##,
        );
        let mut dom = LiveDom::new(&mut rt);
        let frames = dom.list_frames().unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].name, "content");
    }

    #[test]
    fn test_frame_switch() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"exists":true}"#);
        let mut dom = LiveDom::new(&mut rt);
        dom.frame("#iframe1").unwrap();
        assert_eq!(dom.frame_context, "#iframe1");
        dom.default_frame();
        assert!(dom.frame_context.is_empty());
    }

    #[test]
    fn test_frame_not_found() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"exists":false}"#);
        let mut dom = LiveDom::new(&mut rt);
        let err = dom.frame("#nonexistent").unwrap_err();
        assert!(matches!(err, LiveDomError::NotFound(_)));
    }

    // ─── Wait primitives tests ───────────────────────────────────────

    #[test]
    fn test_wait_for_selector_immediate() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"exists":true}"#);
        let mut dom = LiveDom::new(&mut rt);
        let result = dom.wait_for_selector(".modal", 1000).unwrap();
        assert!(result.value);
    }

    #[test]
    fn test_wait_for_stable_immediate() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"length":500}"#);
        let mut dom = LiveDom::new(&mut rt);
        // DOM length stays at 500, so should stabilize in quiet_ms
        let result = dom.wait_for_stable(2000, 150).unwrap();
        assert_eq!(result.outcome, ActionOutcome::NoOp);
    }

    // ─── Legacy compatibility tests ──────────────────────────────────

    #[test]
    fn test_wait_for_legacy() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(r#"{"exists":true}"#);
        let mut dom = LiveDom::new(&mut rt);
        let found = dom.wait_for(".el", 500).unwrap();
        assert!(found);
    }

    // ─── LiveDomResult tests ─────────────────────────────────────────

    #[test]
    fn test_result_with_warnings() {
        let r = LiveDomResult::new("ok".to_string(), ActionOutcome::DomMutation, 5, 10);
        let r = r.with_warnings(vec!["some warning".to_string()]);
        assert_eq!(r.warnings.len(), 1);
        assert_eq!(r.value, "ok");
        assert_eq!(r.mutations, 5);
    }
}
