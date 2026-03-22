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
///
/// Richer than a simple "changed / not changed" — each variant tells the AI
/// caller the *kind* of effect so it can decide what to do next.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ActionOutcome {
    /// Nothing observable changed.
    #[default]
    NoEffect,
    /// DOM structure changed (innerHTML length delta).
    DomOnlyUpdate {
        #[serde(default)]
        mutations: usize,
    },
    /// Input/textarea value changed.
    ValueChanged,
    /// Checkbox was toggled.
    CheckboxToggled {
        #[serde(default)]
        checked: bool,
    },
    /// Radio button was selected.
    RadioSelected {
        #[serde(default)]
        value: String,
    },
    /// Default action was cancelled by `preventDefault()`.
    DefaultActionCancelled,
    /// Constraint validation blocked form submission.
    ValidationBlocked,
    /// Full page navigation (form submit or link click).
    HttpNavigation {
        #[serde(default)]
        url: String,
        #[serde(default)]
        method: String,
    },
    /// SPA route change (pushState / hash change).
    SpaRouteChange {
        #[serde(default)]
        url: String,
    },
    /// A dialog or modal was closed.
    DialogClosed,
    /// A `<details>` element was toggled open/closed.
    ToggleChanged {
        #[serde(default)]
        open: bool,
    },
    /// Focus moved between elements.
    FocusMoved {
        #[serde(default)]
        from: String,
        #[serde(default)]
        to: String,
    },
    /// A new window/tab context was requested (window.open, target=_blank).
    NewContext,
    /// JS state may have changed but no DOM/URL change was detected.
    JsOnlyEffect,
}

/// Per-action trace data returned alongside `ActionOutcome`.
///
/// Only populated when the `NEORENDER_TRACE` env var is set to `1`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActionTrace {
    /// Events dispatched during this action.
    pub events_dispatched: Vec<String>,
    /// What default action (if any) was performed.
    pub default_action: String,
    /// Selector/id of element focused before the action.
    pub focus_before: String,
    /// Selector/id of element focused after the action.
    pub focus_after: String,
    /// Value of target element before the action.
    pub value_before: String,
    /// Value of target element after the action.
    pub value_after: String,
    /// Net change in DOM element count.
    pub dom_delta: i64,
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
    /// Per-action trace (only when NEORENDER_TRACE=1).
    pub trace: Option<ActionTrace>,
}

impl<T> LiveDomResult<T> {
    fn new(value: T, outcome: ActionOutcome, mutations: usize, elapsed_ms: u64) -> Self {
        Self {
            value,
            outcome,
            mutations,
            elapsed_ms,
            warnings: Vec::new(),
            trace: None,
        }
    }

    /// Attach warnings to the result.
    pub fn with_warnings(mut self, warnings: Vec<String>) -> Self {
        self.warnings = warnings;
        self
    }

    /// Attach trace data to the result.
    pub fn with_trace(mut self, trace: ActionTrace) -> Self {
        self.trace = Some(trace);
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
      domLen: document.body ? document.body.innerHTML.length : 0,
      domCount: document.querySelectorAll('*').length,
      focusId: (document.activeElement && document.activeElement.id) || ''
    };
  }

  function detectOutcome(before, after, hint) {
    hint = hint || {};
    if (hint.cancelled) return {kind:'default_action_cancelled'};
    if (hint.validationBlocked) return {kind:'validation_blocked'};
    if (hint.dialogClosed) return {kind:'dialog_closed'};
    if (hint.toggleChanged !== undefined) return {kind:'toggle_changed', open:hint.toggleChanged};
    if (hint.checkboxToggled !== undefined) return {kind:'checkbox_toggled', checked:hint.checkboxToggled};
    if (hint.radioSelected !== undefined) return {kind:'radio_selected', value:hint.radioSelected};
    if (hint.valueChanged) return {kind:'value_changed'};
    if (before.url !== after.url) {
      var a = before.url.split('#')[0];
      var b = after.url.split('#')[0];
      if (a !== b) return {kind:'http_navigation', url:after.url, method:hint.method||'GET'};
      return {kind:'spa_route_change', url:after.url};
    }
    if (before.hash !== after.hash) return {kind:'spa_route_change', url:after.url};
    var delta = Math.abs(after.domLen - before.domLen);
    if (delta > 0) return {kind:'dom_only_update', mutations:delta};
    if (before.focusId !== after.focusId) return {kind:'focus_moved', from:before.focusId, to:after.focusId};
    return {kind:'no_effect'};
  }

  function buildTrace(el, before, after, events, defaultAction) {
    return {
      events_dispatched: events || [],
      default_action: defaultAction || 'none',
      focus_before: before.focusId,
      focus_after: after.focusId,
      value_before: (el && el.value !== undefined) ? String(el._neo_val_before || '') : '',
      value_after: (el && el.value !== undefined) ? String(el.value || '') : '',
      dom_delta: after.domCount - before.domCount
    };
  }

  // ── Event sequences ─────────────────────────────────────────────

  function executeFormSubmit(form, submitter) {
    // F3f: Constraint validation when triggered via button click (not form.submit())
    if (submitter) {
      var formValid = true;
      form.querySelectorAll('input,textarea,select').forEach(function(el) {
        if (el.willValidate && !el.checkValidity()) {
          el.dispatchEvent(new Event('invalid', {bubbles:false}));
          formValid = false;
        }
      });
      if (!formValid) return {action:'validation_blocked'};
    }

    var submitEvt = new Event('submit', {bubbles:false, cancelable:true});
    submitEvt.submitter = submitter || null;
    var prevented = !form.dispatchEvent(submitEvt);

    // React compat: linkedom's event dispatch may not reach React's root
    // delegation. Call React's onSubmit directly if the standard event
    // wasn't handled (no preventDefault called from native listener).
    if (!prevented) {
      var formPropKey = Object.keys(form).find(function(k) { return k.startsWith('__reactProps'); });
      if (formPropKey) {
        var formProps = form[formPropKey];
        if (formProps && typeof formProps.onSubmit === 'function') {
          var synth = {target: form, currentTarget: form, preventDefault: function() { prevented = true; }, stopPropagation: function() {}, type: 'submit', submitter: submitter};
          try { formProps.onSubmit(synth); } catch(e) {}
        }
      }
    }

    if (prevented) return {action:'prevented'};

    // Collect form data
    var data = {};
    var els = form.querySelectorAll('input,select,textarea');
    for (var i = 0; i < els.length; i++) {
      var el = els[i];
      if (!el.name || el.disabled) continue;
      if (el.type === 'checkbox' || el.type === 'radio') {
        if (el.checked) data[el.name] = el.value || 'on';
      } else {
        // Support multiple values with same name (e.g. multi-select)
        if (data[el.name] !== undefined) {
          if (!Array.isArray(data[el.name])) data[el.name] = [data[el.name]];
          data[el.name].push(el.value || '');
        } else {
          data[el.name] = el.value || '';
        }
      }
    }
    // Include submitter value
    if (submitter && submitter.name) data[submitter.name] = submitter.value || '';

    var action = (submitter && submitter.formAction) || form.getAttribute('action') || form.action || location.href;
    var method = ((submitter && submitter.formMethod) || form.getAttribute('method') || form.method || 'GET').toUpperCase();
    var enctype = (submitter && submitter.formEnctype) || form.getAttribute('enctype') || form.enctype || 'application/x-www-form-urlencoded';

    globalThis.__neo_ops.op_navigation_request(JSON.stringify({
      url: action, method: method, form_data: data, enctype: enctype, type: 'form_submit'
    }));
  }

  // ── Focus helper with dirty/change-on-blur (F2e) ───────────────
  function focusElement(target) {
    var prev = document.activeElement;
    if (prev === target) return;
    if (prev && prev !== document.body) {
      if (prev.__neo_dirty) {
        prev.dispatchEvent(new Event('change', {bubbles:true}));
        prev.__neo_dirty = false;
      }
      prev.dispatchEvent(new FocusEvent('focusout', {bubbles:true, relatedTarget:target}));
      prev.dispatchEvent(new FocusEvent('blur', {bubbles:false, relatedTarget:target}));
    }
    if (target && target !== document.body) {
      target.dispatchEvent(new FocusEvent('focusin', {bubbles:true, relatedTarget:prev}));
      target.dispatchEvent(new FocusEvent('focus', {bubbles:false, relatedTarget:prev}));
    }
  }

  // ── Selection helpers (F2d) ────────────────────────────────────
  function getSelStart(el) {
    return typeof el.selectionStart === 'number' ? el.selectionStart : (el.value || '').length;
  }
  function getSelEnd(el) {
    return typeof el.selectionEnd === 'number' ? el.selectionEnd : (el.value || '').length;
  }
  function setCaret(el, pos) {
    el.selectionStart = pos;
    el.selectionEnd = pos;
  }

  // ── Native value setter (React compat) ─────────────────────────
  function getNativeSetter(el) {
    var proto = Object.getPrototypeOf(el);
    return (Object.getOwnPropertyDescriptor(proto, 'value') ||
            Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value') ||
            Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value') ||
            {}).set;
  }
  function setElValue(el, val, setter) {
    // PDR F5: React _valueTracker compat — React compares tracker's cached
    // value with the DOM value to decide whether onChange should fire.
    // We must set the tracker to the OLD value BEFORE updating the DOM,
    // so React detects a delta and fires its synthetic onChange.
    var tracker = el._valueTracker;
    if (tracker && typeof tracker.getValue === 'function') {
      var oldVal = el.value || '';
      tracker.setValue(oldVal);
    }
    if (setter) setter.call(el, val);
    else el.value = val;
  }

  // PDR F5: React controlled input compat.
  // Strategy 1: Re-dispatch an InputEvent after ensuring _valueTracker has the
  // old value cached, so React's root delegation detects the delta.
  // Strategy 2 (fallback): Call __reactProps.onChange / onInput directly.
  function reactNotifyChange(el) {
    // Strategy 1: reset tracker and dispatch — React's SyntheticEvent system
    // listens for native 'input' at the root and compares tracker values.
    var tracker = el._valueTracker;
    if (tracker && typeof tracker.getValue === 'function') {
      // Tracker should already hold the old value from setElValue, but
      // if somehow it got overwritten, force it to empty to guarantee delta.
      var curDom = el.value || '';
      var trackerVal = tracker.getValue();
      if (trackerVal === curDom) {
        // No delta — force one by setting tracker to empty
        tracker.setValue('');
      }
      el.dispatchEvent(new InputEvent('input', {
        bubbles: true, inputType: 'insertText', data: curDom
      }));
    }
    // Strategy 2: direct prop call as last resort (linkedom may not bubble
    // to the React root)
    var propKey = Object.keys(el).find(function(k) { return k.startsWith('__reactProps'); });
    if (!propKey) return;
    var props = el[propKey];
    var syntheticEvt = {target: el, currentTarget: el, type: 'change',
                        nativeEvent: new InputEvent('input', {bubbles:true}),
                        preventDefault: function(){}, stopPropagation: function(){},
                        isDefaultPrevented: function(){ return false; },
                        isPropagationStopped: function(){ return false; }};
    if (props && typeof props.onChange === 'function') {
      try { props.onChange(syntheticEvt); } catch(e) {}
    }
    if (props && typeof props.onInput === 'function') {
      try { props.onInput(syntheticEvt); } catch(e) {}
    }
  }

  function fireClick(el) {
    var events = [];
    var hint = {};
    hint.defaultAction = 'none';

    // Focus sequence (F2e: change-on-blur integrated)
    var prev = document.activeElement;
    if (prev && prev !== el && prev.blur) {
      if (prev.__neo_dirty) {
        prev.dispatchEvent(new Event('change', {bubbles:true}));
        prev.__neo_dirty = false;
      }
      prev.dispatchEvent(new FocusEvent('focusout', {bubbles:true, relatedTarget:el}));
      prev.dispatchEvent(new FocusEvent('blur', {bubbles:false, relatedTarget:el}));
      events.push('focusout','blur');
    }
    if (el.focus) {
      el.dispatchEvent(new FocusEvent('focusin', {bubbles:true, relatedTarget:prev}));
      el.dispatchEvent(new FocusEvent('focus', {bubbles:false, relatedTarget:prev}));
      events.push('focusin','focus');
    }
    // Pointer + mouse sequence
    el.dispatchEvent(new PointerEvent('pointerdown', {bubbles:true, pointerId:1, pointerType:'mouse'}));
    events.push('pointerdown');
    el.dispatchEvent(new MouseEvent('mousedown', {bubbles:true, button:0}));
    events.push('mousedown');
    el.dispatchEvent(new PointerEvent('pointerup', {bubbles:true, pointerId:1, pointerType:'mouse'}));
    events.push('pointerup');
    el.dispatchEvent(new MouseEvent('mouseup', {bubbles:true, button:0}));
    events.push('mouseup');
    var clickEvt = new MouseEvent('click', {bubbles:true, button:0});
    el.dispatchEvent(clickEvt);
    events.push('click');

    // React compat: call onClick directly if present (linkedom bubbling
    // may not reach React's root event delegation).
    var __reactPrevented = false;
    var clickPropKey = Object.keys(el).find(function(k) { return k.startsWith('__reactProps'); });
    if (clickPropKey) {
      var clickProps = el[clickPropKey];
      if (clickProps && typeof clickProps.onClick === 'function') {
        var synth = {target: el, currentTarget: el, preventDefault: function() { __reactPrevented = true; }, stopPropagation: function() {}, type: 'click', button: 0, nativeEvent: clickEvt};
        try { clickProps.onClick(synth); } catch(e) {}
      }
    }

    // Default actions (only if not prevented)
    if (!clickEvt.defaultPrevented && !__reactPrevented) {
      var tag = (el.tagName || '').toUpperCase();
      if (tag === 'A' && el.href) {
        var href = el.getAttribute('href') || '';
        if (href.startsWith('#')) { /* hash nav */ }
        else if (href.startsWith('javascript:')) { try { eval(href.slice(11)); } catch(e) {} }
        else if (el.target === '_blank') { hint.defaultAction = 'new_context'; }
        else if (el.hasAttribute('download')) { hint.defaultAction = 'download'; }
        else {
          globalThis.__neo_ops.op_navigation_request(JSON.stringify({url:el.href,method:'GET',type:'link_click'}));
          hint.defaultAction = 'navigation'; hint.method = 'GET';
        }
      }
      else if (tag === 'INPUT' && el.type === 'checkbox') {
        el.checked = !el.checked;
        el.dispatchEvent(new InputEvent('input', {bubbles:true}));
        el.dispatchEvent(new Event('change', {bubbles:true}));
        events.push('input','change');
        hint.defaultAction = 'checkbox_toggle'; hint.checkboxToggled = el.checked;
      }
      else if (tag === 'INPUT' && el.type === 'radio') {
        var form = el.form || el.closest('form') || document;
        var group = form.querySelectorAll('input[type=radio][name="' + el.name + '"]');
        group.forEach(function(r) { if (r !== el) r.checked = false; });
        el.checked = true;
        el.dispatchEvent(new InputEvent('input', {bubbles:true}));
        el.dispatchEvent(new Event('change', {bubbles:true}));
        events.push('input','change');
        hint.defaultAction = 'radio_select'; hint.radioSelected = el.value || '';
      }
      else if ((tag === 'BUTTON' && (el.type === 'submit' || !el.type || el.type === ''))
              || (tag === 'INPUT' && el.type === 'submit')) {
        var form = el.closest('form');
        if (form) {
          var submitResult = executeFormSubmit(form, el);
          if (submitResult && submitResult.action === 'validation_blocked') {
            hint.defaultAction = 'validation_blocked'; hint.validationBlocked = true;
          } else {
            hint.defaultAction = 'form_submit';
            hint.method = ((form.getAttribute('method') || 'GET').toUpperCase());
          }
        }
      }
      else if (tag === 'LABEL') {
        var forId = el.getAttribute('for');
        var ctrl = forId ? document.getElementById(forId) : el.querySelector('input,select,textarea');
        if (ctrl && ctrl !== el) {
          var sub = fireClick(ctrl);
          hint = sub.hint || hint;
          events = events.concat(sub.events || []);
        }
      }
      else if (tag === 'SUMMARY') {
        var details = el.closest('details');
        if (details) {
          var wasOpen = details.hasAttribute('open');
          if (wasOpen) details.removeAttribute('open');
          else details.setAttribute('open', '');
          hint.defaultAction = 'toggle'; hint.toggleChanged = !wasOpen;
        }
      }
    } else {
      hint.cancelled = true; hint.defaultAction = 'cancelled';
    }

    return { events: events, hint: hint };
  }

  function fireTypeText(el, text) {
    // Focus if not already focused
    if (document.activeElement !== el && el.focus) el.focus();

    // F3b: Select handling — set value + dispatch change instead of typing
    if (el.tagName === 'SELECT') {
      if (el.multiple) {
        // select-multiple: text is comma-separated values
        var vals = text.split(',');
        for (var si = 0; si < el.options.length; si++) {
          var match = vals.indexOf(el.options[si].value) >= 0;
          el.options[si].selected = match;
          // linkedom: also set/remove attribute since property may not stick
          if (match) el.options[si].setAttribute('selected', '');
          else el.options[si].removeAttribute('selected');
        }
      } else {
        // select-one: set selected on matching option via both property and attribute
        for (var si = 0; si < el.options.length; si++) {
          var match = (el.options[si].value === text);
          el.options[si].selected = match;
          if (match) el.options[si].setAttribute('selected', '');
          else el.options[si].removeAttribute('selected');
        }
        try { el.value = text; } catch(e) { /* linkedom: value may be read-only */ }
      }
      el.dispatchEvent(new InputEvent('input', {bubbles:true}));
      el.dispatchEvent(new Event('change', {bubbles:true}));
      return;
    }

    var setter = getNativeSetter(el);

    // F2d: initialize caret at end of existing value if not already set
    if (typeof el._selStart !== 'number') {
      setCaret(el, (el.value || '').length);
    }

    for (var i = 0; i < text.length; i++) {
      var ch = text[i];
      var code = ch >= 'a' && ch <= 'z' ? 'Key' + ch.toUpperCase()
               : ch >= 'A' && ch <= 'Z' ? 'Key' + ch
               : ch >= '0' && ch <= '9' ? 'Digit' + ch
               : ch === ' ' ? 'Space' : 'Key' + ch;

      el.dispatchEvent(new KeyboardEvent('keydown', {key:ch, code:code, bubbles:true}));
      el.dispatchEvent(new KeyboardEvent('keypress', {key:ch, code:code, bubbles:true, charCode:ch.charCodeAt(0)}));

      // F2c: beforeinput — cancelable
      var bi = new InputEvent('beforeinput', {inputType:'insertText', data:ch, cancelable:true, bubbles:true});
      el.dispatchEvent(bi);
      if (bi.defaultPrevented) {
        el.dispatchEvent(new KeyboardEvent('keyup', {key:ch, code:code, bubbles:true}));
        continue;
      }

      // F2d+F2f: insert at caret, replacing selection if any
      var curVal = el.value || '';
      var selS = getSelStart(el);
      var selE = getSelEnd(el);
      var before = curVal.substring(0, selS);
      var after = curVal.substring(selE);
      var newVal = before + ch + after;
      setElValue(el, newVal, setter);
      setCaret(el, selS + 1);

      // F2e: mark dirty
      el.__neo_dirty = true;

      el.dispatchEvent(new InputEvent('input', {data:ch, inputType:'insertText', bubbles:true}));
      el.dispatchEvent(new KeyboardEvent('keyup', {key:ch, code:code, bubbles:true}));
    }
    // React compat: notify React of the final value (linkedom bubbling may not
    // reach React's root delegation, so call onChange directly).
    reactNotifyChange(el);
    // F2e: NO change event here — fires on blur
  }

  function fireSubmit(form) {
    // Blur active element first (F2e: triggers change if dirty)
    var active = document.activeElement;
    if (active && active !== document.body) {
      if (active.__neo_dirty) {
        active.dispatchEvent(new Event('change', {bubbles:true}));
        active.__neo_dirty = false;
      }
      if (active.blur) active.blur();
    }
    executeFormSubmit(form, null);
    return true;
  }

  function firePressKey(el, key) {
    var _events = [];
    var _hint = {};
    _hint.defaultAction = 'none';
    var opts = { key: key, code: key, bubbles: true, cancelable: true };

    // F2a: Tab focus cycling
    if (key === 'Tab' || key === 'Shift+Tab') {
      var shift = key === 'Shift+Tab';
      el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', code: 'Tab', bubbles: true, cancelable: true, shiftKey: shift }));
      var focusable = Array.from(document.querySelectorAll(
        'input:not([disabled]):not([type=hidden]), select:not([disabled]), textarea:not([disabled]), button:not([disabled]), a[href], [tabindex]'
      )).filter(function(e) {
        var ti = parseInt(e.getAttribute('tabindex'), 10);
        return isNaN(ti) || ti >= 0;
      });
      focusable.sort(function(a, b) {
        var ta = parseInt(a.getAttribute('tabindex'), 10) || 0;
        var tb = parseInt(b.getAttribute('tabindex'), 10) || 0;
        if (ta > 0 && tb > 0) return ta - tb;
        if (ta > 0) return -1;
        if (tb > 0) return 1;
        return 0;
      });
      if (focusable.length > 0) {
        var current = document.activeElement;
        var idx = focusable.indexOf(current);
        var next = shift ? idx - 1 : idx + 1;
        if (next >= focusable.length) next = 0;
        if (next < 0) next = focusable.length - 1;
        focusElement(focusable[next]);
      }
      el.dispatchEvent(new KeyboardEvent('keyup', { key: 'Tab', code: 'Tab', bubbles: true, shiftKey: shift }));
      return { submitted: false, events: _events, hint: _hint };
    }

    el.dispatchEvent(new KeyboardEvent('keydown', opts));
    _events.push('keydown');

    // F2b: Backspace
    if (key === 'Backspace' && el.value !== undefined) {
      var curVal = el.value || '';
      var selS = getSelStart(el);
      var selE = getSelEnd(el);
      var bi = new InputEvent('beforeinput', {inputType:'deleteContentBackward', data:null, cancelable:true, bubbles:true});
      el.dispatchEvent(bi);
      if (!bi.defaultPrevented) {
        var bsSetter = getNativeSetter(el);
        if (selS !== selE) {
          var nv = curVal.substring(0, selS) + curVal.substring(selE);
          setElValue(el, nv, bsSetter);
          setCaret(el, selS);
        } else if (selS > 0) {
          var nv = curVal.substring(0, selS - 1) + curVal.substring(selS);
          setElValue(el, nv, bsSetter);
          setCaret(el, selS - 1);
        }
        el.__neo_dirty = true;
        el.dispatchEvent(new InputEvent('input', {inputType:'deleteContentBackward', bubbles:true}));
      }
      el.dispatchEvent(new KeyboardEvent('keyup', opts));
    _events.push('keyup');
      return { submitted: false, events: _events, hint: _hint };
    }

    // F2b: Delete key
    if (key === 'Delete' && el.value !== undefined) {
      var curVal = el.value || '';
      var selS = getSelStart(el);
      var selE = getSelEnd(el);
      var bi = new InputEvent('beforeinput', {inputType:'deleteContentForward', data:null, cancelable:true, bubbles:true});
      el.dispatchEvent(bi);
      if (!bi.defaultPrevented) {
        var delSetter = getNativeSetter(el);
        if (selS !== selE) {
          var nv = curVal.substring(0, selS) + curVal.substring(selE);
          setElValue(el, nv, delSetter);
          setCaret(el, selS);
        } else if (selS < curVal.length) {
          var nv = curVal.substring(0, selS) + curVal.substring(selS + 1);
          setElValue(el, nv, delSetter);
          setCaret(el, selS);
        }
        el.__neo_dirty = true;
        el.dispatchEvent(new InputEvent('input', {inputType:'deleteContentForward', bubbles:true}));
      }
      el.dispatchEvent(new KeyboardEvent('keyup', opts));
    _events.push('keyup');
      return { submitted: false, events: _events, hint: _hint };
    }

    el.dispatchEvent(new KeyboardEvent('keyup', opts));
    _events.push('keyup');

    // Enter: submit form or insertLineBreak for textarea
    if (key === 'Enter') {
      var tag = (el.tagName || '').toUpperCase();
      if (tag === 'TEXTAREA') {
        var bi = new InputEvent('beforeinput', {inputType:'insertLineBreak', data:null, cancelable:true, bubbles:true});
        el.dispatchEvent(bi);
        if (!bi.defaultPrevented) {
          var enterSetter = getNativeSetter(el);
          var curVal = el.value || '';
          var selS = getSelStart(el);
          var selE = getSelEnd(el);
          var nv = curVal.substring(0, selS) + '\n' + curVal.substring(selE);
          setElValue(el, nv, enterSetter);
          setCaret(el, selS + 1);
          el.__neo_dirty = true;
          el.dispatchEvent(new InputEvent('input', {inputType:'insertLineBreak', bubbles:true}));
        }
        return { submitted: false, events: _events, hint: _hint };
      }
      var form = el.closest ? el.closest('form') : null;
      if (form) {
        var submitResult = executeFormSubmit(form, null);
        if (submitResult && submitResult.action === 'validation_blocked') {
          _hint.defaultAction = 'validation_blocked'; _hint.validationBlocked = true;
        } else {
          _hint.defaultAction = 'form_submit'; _hint.method = ((form.getAttribute('method') || 'GET').toUpperCase());
        }
        return { submitted: true, events: _events, hint: _hint };
      }
    }

    // F3e: Escape dismiss
    if (key === 'Escape') {
      var dialog = document.querySelector('dialog[open]');
      if (dialog) {
        dialog.removeAttribute('open');
        dialog.dispatchEvent(new Event('close'));
        _hint.defaultAction = 'dialog_closed'; _hint.dialogClosed = true; return { submitted: false, events: _events, hint: _hint };
      }
      var roleDialog = document.querySelector('[role=dialog][aria-modal=true]');
      if (roleDialog) {
        roleDialog.style.display = 'none';
        _hint.defaultAction = 'dialog_closed'; _hint.dialogClosed = true; return { submitted: false, events: _events, hint: _hint };
      }
    }

    return { submitted: false, events: _events, hint: _hint };
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
          r.el._neo_val_before = r.el.value || '';
          var clickInfo = fireClick(r.el);
          var after = snapshot();
          var mutations = Math.abs(after.domLen - before.domLen);
          var outcomeObj = detectOutcome(before, after, clickInfo.hint);
          var result = {
            ok: true,
            text: (r.el.textContent || '').trim().substring(0, 100),
            href: r.el.href || '',
            outcome: outcomeObj,
            mutations: mutations,
            count: r.count
          };
          result.trace = buildTrace(r.el, before, after, clickInfo.events, clickInfo.hint.defaultAction);
          return JSON.stringify(result);
        }

        case 'type_text': {
          var r = resolve(selector, { interactable: true });
          if (r.error) return JSON.stringify(r);
          var valBefore = r.el.value || '';
          r.el._neo_val_before = valBefore;
          fireTypeText(r.el, value);
          var after = snapshot();
          var typeHint = {valueChanged: (r.el.value || '') !== valBefore};
          var outcomeObj = detectOutcome(before, after, typeHint);
          var result = {
            ok: true,
            outcome: outcomeObj,
            mutations: Math.abs(after.domLen - before.domLen),
            count: r.count
          };
          result.trace = buildTrace(r.el, before, after, ['keydown','keypress','input','keyup'], 'none');
          return JSON.stringify(result);
        }

        case 'press_key': {
          var r = resolve(selector);
          if (r.error) {
            var active = document.activeElement;
            if (!active) return JSON.stringify(r);
            active._neo_val_before = active.value || '';
            var info = firePressKey(active, key);
            var after = snapshot();
            var outcomeObj = detectOutcome(before, after, info.hint);
            var result = {
              ok: true,
              submitted: info.submitted,
              outcome: outcomeObj,
              mutations: Math.abs(after.domLen - before.domLen)
            };
            result.trace = buildTrace(active, before, after, info.events, info.hint.defaultAction);
            return JSON.stringify(result);
          }
          r.el._neo_val_before = r.el.value || '';
          var info = firePressKey(r.el, key);
          var after = snapshot();
          var outcomeObj = detectOutcome(before, after, info.hint);
          var result = {
            ok: true,
            submitted: info.submitted,
            outcome: outcomeObj,
            mutations: Math.abs(after.domLen - before.domLen),
            count: r.count
          };
          result.trace = buildTrace(r.el, before, after, info.events, info.hint.defaultAction);
          return JSON.stringify(result);
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
            var submitHint = {method: ((form.getAttribute('method') || 'GET').toUpperCase())};
            var outcomeObj = detectOutcome(before, after, submitHint);
            return JSON.stringify({
              ok: true,
              action: action_url,
              clicked: false,
              submitted: submitted,
              outcome: outcomeObj,
              mutations: Math.abs(after.domLen - before.domLen)
            });
          }
          // No form — click the element as fallback
          var clickInfo = fireClick(el);
          var after = snapshot();
          var outcomeObj = detectOutcome(before, after, clickInfo.hint);
          return JSON.stringify({
            ok: true,
            action: '',
            clicked: true,
            submitted: false,
            outcome: outcomeObj,
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

        case 'find': {
          // Smart element finder: CSS, text, ARIA, placeholder, name, fuzzy
          var query = selector;
          var found = [];
          var seen = [];

          function addEl(el) {
            if (seen.indexOf(el) >= 0) return;
            seen.push(el);
            var tag = (el.tagName || '').toLowerCase();
            // Build unique selector
            var usel = '';
            if (el.id) { usel = '#' + el.id; }
            else {
              var p = el.parentElement;
              if (p) {
                var siblings = p.querySelectorAll(':scope > ' + tag);
                if (siblings.length === 1) {
                  usel = tag;
                } else {
                  for (var si = 0; si < siblings.length; si++) {
                    if (siblings[si] === el) { usel = tag + ':nth-of-type(' + (si+1) + ')'; break; }
                  }
                }
                // Prepend parent info for uniqueness
                if (p.id) usel = '#' + p.id + ' > ' + usel;
                else if (p.tagName) usel = (p.tagName||'').toLowerCase() + ' > ' + usel;
              } else {
                usel = tag;
              }
              // If still not unique, add class
              if (el.className && typeof el.className === 'string') {
                var cls = el.className.trim().split(/\s+/)[0];
                if (cls) usel += '.' + cls;
              }
            }
            // Infer role
            var role = el.getAttribute('role') || '';
            if (!role) {
              if (tag === 'a') role = 'link';
              else if (tag === 'button') role = 'button';
              else if (tag === 'input') role = el.type === 'checkbox' ? 'checkbox' : el.type === 'radio' ? 'radio' : 'textbox';
              else if (tag === 'textarea') role = 'textbox';
              else if (tag === 'select') role = 'combobox';
              else if (tag === 'img') role = 'img';
              else if (tag.match(/^h[1-6]$/)) role = 'heading';
              else role = 'generic';
            }
            // Best label
            var label = el.getAttribute('aria-label')
              || el.getAttribute('placeholder')
              || el.getAttribute('title')
              || (el.labels && el.labels[0] ? (el.labels[0].textContent||'').trim() : '')
              || (el.textContent || '').trim().substring(0, 60)
              || '';
            // Element type
            var etype = '';
            if (tag === 'input') etype = el.type || 'text';
            else if (tag === 'button') etype = 'button';
            else if (tag === 'a') etype = 'link';
            else if (tag === 'select') etype = 'select';
            else if (tag === 'textarea') etype = 'textarea';
            else etype = tag;
            // Value
            var val = '';
            if ('value' in el && (tag === 'input' || tag === 'textarea' || tag === 'select')) {
              val = String(el.value || '');
            }
            // Interactable
            var inter = isVisible(el) && isEnabled(el);

            found.push({
              selector: usel,
              tag: tag,
              role: role,
              label: label.substring(0, 120),
              element_type: etype,
              value: val,
              interactable: inter
            });
          }

          // Strategy 1: CSS selector
          try {
            var byCSS = document.querySelectorAll(query);
            for (var ci = 0; ci < byCSS.length && ci < 20; ci++) addEl(byCSS[ci]);
          } catch(e) {}

          // Strategy 2: text content (exact then fuzzy)
          var lowerQ = query.toLowerCase();
          var allText = document.querySelectorAll('a, button, [role="button"], label, span, p, h1, h2, h3, h4, h5, h6, li, td, th, div, input, textarea, select');
          for (var ti = 0; ti < allText.length && found.length < 20; ti++) {
            var el = allText[ti];
            var t = (el.textContent || '').trim().toLowerCase();
            if (t === lowerQ) addEl(el);
          }

          // Strategy 3: ARIA label match
          var byAriaLabel = document.querySelectorAll('[aria-label]');
          for (var ai = 0; ai < byAriaLabel.length && found.length < 20; ai++) {
            var ariaVal = (byAriaLabel[ai].getAttribute('aria-label') || '').toLowerCase();
            if (ariaVal === lowerQ || ariaVal.indexOf(lowerQ) >= 0) addEl(byAriaLabel[ai]);
          }

          // Strategy 4: placeholder match
          var byPlaceholder = document.querySelectorAll('[placeholder]');
          for (var pi = 0; pi < byPlaceholder.length && found.length < 20; pi++) {
            var phVal = (byPlaceholder[pi].getAttribute('placeholder') || '').toLowerCase();
            if (phVal === lowerQ || phVal.indexOf(lowerQ) >= 0) addEl(byPlaceholder[pi]);
          }

          // Strategy 5: name attribute match
          var byName = document.querySelectorAll('[name]');
          for (var ni = 0; ni < byName.length && found.length < 20; ni++) {
            var nameVal = (byName[ni].getAttribute('name') || '').toLowerCase();
            if (nameVal === lowerQ || nameVal.indexOf(lowerQ) >= 0) addEl(byName[ni]);
          }

          // Strategy 6: fuzzy text contains (case-insensitive)
          if (found.length === 0) {
            for (var fi = 0; fi < allText.length && found.length < 20; fi++) {
              var ft = (allText[fi].textContent || '').trim().toLowerCase();
              if (ft.indexOf(lowerQ) >= 0 && ft.length < lowerQ.length * 4) addEl(allText[fi]);
            }
          }

          return JSON.stringify(found);
        }

        case 'fill_smart': {
          // Smart form fill: fields is a JSON object { label: value, ... }
          // Finds each field by name/label/placeholder/aria-label, then sets value React-compatible
          var fieldsObj;
          try { fieldsObj = JSON.parse(value); } catch(e) {
            return JSON.stringify({ error: 'parse', message: 'fields must be JSON object' });
          }
          var results = [];
          var keys = Object.keys(fieldsObj);
          for (var fi = 0; fi < keys.length; fi++) {
            var fieldKey = keys[fi];
            var fieldVal = fieldsObj[fieldKey];
            var el = null;
            var lk = fieldKey.toLowerCase();

            // Try name attribute first
            var byName = document.querySelector('[name="' + fieldKey + '"]')
                      || document.querySelector('[name="' + fieldKey.toLowerCase() + '"]');
            if (byName) el = byName;

            // Try id
            if (!el) {
              try { el = document.getElementById(fieldKey); } catch(e) {}
            }

            // Try placeholder
            if (!el) {
              var allInputs = document.querySelectorAll('input, textarea, select');
              for (var ii = 0; ii < allInputs.length; ii++) {
                var ph = (allInputs[ii].getAttribute('placeholder') || '').toLowerCase();
                if (ph === lk || ph.indexOf(lk) >= 0) { el = allInputs[ii]; break; }
              }
            }

            // Try aria-label
            if (!el) {
              var allInputs2 = document.querySelectorAll('input, textarea, select');
              for (var ii2 = 0; ii2 < allInputs2.length; ii2++) {
                var al = (allInputs2[ii2].getAttribute('aria-label') || '').toLowerCase();
                if (al === lk || al.indexOf(lk) >= 0) { el = allInputs2[ii2]; break; }
              }
            }

            // Try associated label
            if (!el) {
              var labels = document.querySelectorAll('label');
              for (var li = 0; li < labels.length; li++) {
                var lt = (labels[li].textContent || '').trim().toLowerCase();
                if (lt === lk || lt.indexOf(lk) >= 0) {
                  var forId = labels[li].getAttribute('for');
                  if (forId) { el = document.getElementById(forId); }
                  else { el = labels[li].querySelector('input, textarea, select'); }
                  if (el) break;
                }
              }
            }

            if (!el) {
              results.push({ field: fieldKey, ok: false, error: 'not_found' });
              continue;
            }

            // Set value React-compatible
            var tag = (el.tagName || '').toUpperCase();
            if (tag === 'SELECT') {
              // Set selectedIndex
              for (var oi = 0; oi < el.options.length; oi++) {
                var match = (el.options[oi].value === fieldVal) || (el.options[oi].textContent || '').trim() === fieldVal;
                el.options[oi].selected = match;
                if (match) el.options[oi].setAttribute('selected', '');
                else el.options[oi].removeAttribute('selected');
              }
              try { el.value = fieldVal; } catch(e) {}
              el.dispatchEvent(new InputEvent('input', {bubbles:true}));
              el.dispatchEvent(new Event('change', {bubbles:true}));
              reactNotifyChange(el);
              results.push({ field: fieldKey, ok: true, tag: tag.toLowerCase() });
            }
            else if (tag === 'INPUT' && (el.type === 'checkbox' || el.type === 'radio')) {
              var shouldCheck = fieldVal === 'true' || fieldVal === '1' || fieldVal === 'on' || fieldVal === 'yes';
              if (el.checked !== shouldCheck) {
                el.checked = shouldCheck;
                el.dispatchEvent(new InputEvent('input', {bubbles:true}));
                el.dispatchEvent(new Event('change', {bubbles:true}));
              }
              results.push({ field: fieldKey, ok: true, tag: el.type });
            }
            else {
              // Text input / textarea — use React-compatible setter
              var setter = getNativeSetter(el);
              setElValue(el, fieldVal, setter);
              el.dispatchEvent(new InputEvent('input', {bubbles:true, inputType:'insertText', data:fieldVal}));
              el.dispatchEvent(new Event('change', {bubbles:true}));
              reactNotifyChange(el);
              results.push({ field: fieldKey, ok: true, tag: tag.toLowerCase() });
            }
          }
          return JSON.stringify({ ok: true, fields: results });
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
    /// Structured outcome object from detectOutcome (F4c).
    #[serde(default)]
    outcome: Option<serde_json::Value>,
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
    /// Per-action trace (F4b) — populated when NEORENDER_TRACE=1.
    #[serde(default)]
    trace: Option<TraceResponse>,
}

/// Raw trace data from JS dispatcher.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TraceResponse {
    #[serde(default)]
    events_dispatched: Vec<String>,
    #[serde(default)]
    default_action: String,
    #[serde(default)]
    focus_before: String,
    #[serde(default)]
    focus_after: String,
    #[serde(default)]
    value_before: String,
    #[serde(default)]
    value_after: String,
    #[serde(default)]
    dom_delta: i64,
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

/// Parse a structured outcome object from the JS dispatcher.
///
/// Handles both the new object format `{kind: "...", ...}` and legacy
/// string format `"dom_mutation"` for backward compatibility with mocks.
fn parse_outcome_value(v: &serde_json::Value) -> ActionOutcome {
    // Legacy: if the value is a plain string, treat it as the kind directly
    let kind = if let Some(s) = v.as_str() {
        s
    } else {
        v.get("kind").and_then(|k| k.as_str()).unwrap_or("no_effect")
    };
    match kind {
        "no_effect" => ActionOutcome::NoEffect,
        "dom_only_update" => ActionOutcome::DomOnlyUpdate {
            mutations: v.get("mutations").and_then(|m| m.as_u64()).unwrap_or(0) as usize,
        },
        "value_changed" => ActionOutcome::ValueChanged,
        "checkbox_toggled" => ActionOutcome::CheckboxToggled {
            checked: v.get("checked").and_then(|c| c.as_bool()).unwrap_or(false),
        },
        "radio_selected" => ActionOutcome::RadioSelected {
            value: v.get("value").and_then(|s| s.as_str()).unwrap_or("").to_string(),
        },
        "default_action_cancelled" => ActionOutcome::DefaultActionCancelled,
        "validation_blocked" => ActionOutcome::ValidationBlocked,
        "http_navigation" => ActionOutcome::HttpNavigation {
            url: v.get("url").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            method: v.get("method").and_then(|s| s.as_str()).unwrap_or("GET").to_string(),
        },
        "spa_route_change" => ActionOutcome::SpaRouteChange {
            url: v.get("url").and_then(|s| s.as_str()).unwrap_or("").to_string(),
        },
        "dialog_closed" => ActionOutcome::DialogClosed,
        "toggle_changed" => ActionOutcome::ToggleChanged {
            open: v.get("open").and_then(|o| o.as_bool()).unwrap_or(false),
        },
        "focus_moved" => ActionOutcome::FocusMoved {
            from: v.get("from").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            to: v.get("to").and_then(|s| s.as_str()).unwrap_or("").to_string(),
        },
        "new_context" => ActionOutcome::NewContext,
        "js_only_effect" => ActionOutcome::JsOnlyEffect,
        // Legacy string-based outcomes (backward compat with mocks/old format)
        "no_op" => ActionOutcome::NoEffect,
        "dom_mutation" => ActionOutcome::DomOnlyUpdate { mutations: 0 },
        "full_navigation" => ActionOutcome::HttpNavigation {
            url: String::new(),
            method: String::new(),
        },
        "spa_route" => ActionOutcome::SpaRouteChange {
            url: String::new(),
        },
        _ => ActionOutcome::NoEffect,
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
        // Build JSON cmd and pass via serde to avoid escaping issues with quotes.
        let cmd = serde_json::json!({
            "action": action,
            "selector": selector,
            "value": value,
            "key": key,
        });
        let cmd_str = cmd.to_string();
        // Escape for JS template literal (backticks)
        let safe = cmd_str.replace('\\', "\\\\").replace('`', "\\`").replace("${", "\\${");
        let js = format!("window.__neo.exec(`{}`)", safe);
        let raw = self.runtime.eval(&js)?;
        serde_json::from_str(&raw).map_err(|e| {
            LiveDomError::Parse(format!("{e}: raw={}", &raw[..raw.len().min(200)]))
        })
    }

    /// Execute a dispatcher command for actions that return raw strings (page_text, etc.).
    fn dispatch_raw(&mut self, action: &str, selector: &str, value: &str, key: &str) -> Result<String, LiveDomError> {
        self.ensure_dispatcher()?;
        let cmd = serde_json::json!({
            "action": action,
            "selector": selector,
            "value": value,
            "key": key,
        });
        let cmd_str = cmd.to_string();
        let safe = cmd_str.replace('\\', "\\\\").replace('`', "\\`").replace("${", "\\${");
        let js = format!("window.__neo.exec(`{}`)", safe);
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

    /// Extract outcome, mutations, and optional trace from a dispatch response.
    fn extract_outcome(resp: &DispatchResponse) -> (ActionOutcome, usize, Option<ActionTrace>) {
        let outcome = resp
            .outcome
            .as_ref()
            .map(parse_outcome_value)
            .unwrap_or_default();
        let mutations = resp.mutations.unwrap_or(0);
        let trace = resp.trace.as_ref().map(|t| ActionTrace {
            events_dispatched: t.events_dispatched.clone(),
            default_action: t.default_action.clone(),
            focus_before: t.focus_before.clone(),
            focus_after: t.focus_after.clone(),
            value_before: t.value_before.clone(),
            value_after: t.value_after.clone(),
            dom_delta: t.dom_delta,
        });
        (outcome, mutations, trace)
    }

    // ─── Public API ──────────────────────────────────────────────────

    /// Click an element. Returns the element's text and href.
    pub fn click(&mut self, selector: &str) -> Result<LiveDomResult<String>, LiveDomError> {
        let start = Instant::now();
        let resp = self.dispatch("click", selector, "", "")?;
        if let Some(e) = Self::check_error(&resp) {
            return Err(e);
        }
        let (outcome, mutations, trace) = Self::extract_outcome(&resp);
        let text = resp.text.unwrap_or_default();
        let href = resp.href.unwrap_or_default();
        let mut result = text.clone();
        if !href.is_empty() {
            result = format!("{text} -> {href}");
        }
        let elapsed = start.elapsed().as_millis() as u64;
        let mut r = LiveDomResult::new(result, outcome, mutations, elapsed);
        if let Some(t) = trace {
            r = r.with_trace(t);
        }
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
        let (outcome, mutations, trace) = Self::extract_outcome(&resp);
        let elapsed = start.elapsed().as_millis() as u64;
        let mut r = LiveDomResult::new((), outcome, mutations, elapsed);
        if let Some(t) = trace {
            r = r.with_trace(t);
        }
        Ok(r)
    }

    /// Press a special key (Enter, Tab, Escape, etc.).
    pub fn press_key(&mut self, selector: &str, key: &str) -> Result<LiveDomResult<()>, LiveDomError> {
        let start = Instant::now();
        let resp = self.dispatch("press_key", selector, "", key)?;
        if let Some(e) = Self::check_error(&resp) {
            return Err(e);
        }
        let (outcome, mutations, trace) = Self::extract_outcome(&resp);
        let elapsed = start.elapsed().as_millis() as u64;
        let mut r = LiveDomResult::new((), outcome, mutations, elapsed);
        if let Some(t) = trace {
            r = r.with_trace(t);
        }
        Ok(r)
    }

    /// Submit a form (finds closest `<form>` and submits, or clicks the element).
    pub fn submit(&mut self, selector: &str) -> Result<LiveDomResult<String>, LiveDomError> {
        let start = Instant::now();
        let resp = self.dispatch("submit", selector, "", "")?;
        if let Some(e) = Self::check_error(&resp) {
            return Err(e);
        }
        let (outcome, mutations, trace) = Self::extract_outcome(&resp);
        let elapsed = start.elapsed().as_millis() as u64;
        let value = if resp.clicked.unwrap_or(false) {
            "clicked".to_string()
        } else {
            resp.action.unwrap_or_default()
        };
        let mut r = LiveDomResult::new(value, outcome, mutations, elapsed);
        if let Some(t) = trace {
            r = r.with_trace(t);
        }
        Ok(r)
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
            ActionOutcome::NoEffect,
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
            ActionOutcome::NoEffect,
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
                return Ok(LiveDomResult::new(true, ActionOutcome::NoEffect, 0, elapsed));
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        // Final check
        if self.exists(selector)? {
            let elapsed = start.elapsed().as_millis() as u64;
            return Ok(LiveDomResult::new(true, ActionOutcome::NoEffect, 0, elapsed));
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
                    return Ok(LiveDomResult::new(true, ActionOutcome::NoEffect, 0, elapsed));
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
                return Ok(LiveDomResult::new((), ActionOutcome::NoEffect, 0, total_elapsed));
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
        let mut last_outcome = ActionOutcome::NoEffect;
        for (selector, value) in fields {
            let r = self.type_text(selector, value)?;
            total_mutations = total_mutations.saturating_add(r.mutations);
            if r.outcome != ActionOutcome::NoEffect {
                last_outcome = r.outcome;
            }
        }
        let elapsed = start.elapsed().as_millis() as u64;
        Ok(LiveDomResult::new((), last_outcome, total_mutations, elapsed))
    }

    /// Smart element finder — searches by CSS, text, ARIA, placeholder, name, fuzzy.
    pub fn find_element(&mut self, query: &str) -> Result<Vec<crate::FoundElement>, LiveDomError> {
        let raw = self.dispatch_raw("find", query, "", "")?;
        let elements: Vec<crate::FoundElement> = serde_json::from_str(&raw)
            .map_err(|e| LiveDomError::Parse(format!("{e}: raw={}", &raw[..raw.len().min(200)])))?;
        Ok(elements)
    }

    /// Smart form fill — finds fields by name/label/placeholder/aria-label, fills React-compatible.
    pub fn fill_form_smart(&mut self, fields: &std::collections::HashMap<String, String>) -> Result<(), LiveDomError> {
        let fields_json = serde_json::to_string(fields)
            .map_err(|e| LiveDomError::Parse(format!("serialize fields: {e}")))?;
        let raw = self.dispatch_raw("fill_smart", "", &fields_json, "")?;
        // Check for errors in response
        let resp: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| LiveDomError::Parse(format!("{e}: raw={}", &raw[..raw.len().min(200)])))?;
        if let Some(err) = resp.get("error").and_then(|e| e.as_str()) {
            let msg = resp.get("message").and_then(|m| m.as_str()).unwrap_or(err);
            return Err(LiveDomError::JsException(msg.to_string()));
        }
        // Check individual field results
        if let Some(fields_arr) = resp.get("fields").and_then(|f| f.as_array()) {
            let failed: Vec<String> = fields_arr.iter()
                .filter(|f| f.get("ok").and_then(|o| o.as_bool()) != Some(true))
                .filter_map(|f| f.get("field").and_then(|n| n.as_str()).map(String::from))
                .collect();
            if !failed.is_empty() {
                return Err(LiveDomError::NotFound(format!("fields not found: {}", failed.join(", "))));
            }
        }
        Ok(())
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
        assert_eq!(result.outcome, ActionOutcome::DomOnlyUpdate { mutations: 0 });
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
        assert_eq!(result.outcome, ActionOutcome::DomOnlyUpdate { mutations: 0 });
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
        assert_eq!(result.outcome, ActionOutcome::HttpNavigation { url: String::new(), method: String::new() });
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
        assert_eq!(result.outcome, ActionOutcome::NoEffect);
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
        assert_eq!(result.outcome, ActionOutcome::DomOnlyUpdate { mutations: 0 });
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
        assert_eq!(result.outcome, ActionOutcome::SpaRouteChange { url: String::new() });
    }

    #[test]
    fn test_outcome_full_navigation() {
        let mut rt = MockRuntime::new();
        rt.set_default_eval(
            r#"{"ok":true,"text":"Go","href":"","outcome":"full_navigation","mutations":0}"#,
        );
        let mut dom = LiveDom::new(&mut rt);
        let result = dom.click("a.external").unwrap();
        assert_eq!(result.outcome, ActionOutcome::HttpNavigation { url: String::new(), method: String::new() });
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
        assert_eq!(result.outcome, ActionOutcome::NoEffect);
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
        let r = LiveDomResult::new("ok".to_string(), ActionOutcome::DomOnlyUpdate { mutations: 0 }, 5, 10);
        let r = r.with_warnings(vec!["some warning".to_string()]);
        assert_eq!(r.warnings.len(), 1);
        assert_eq!(r.value, "ok");
        assert_eq!(r.mutations, 5);
    }
}
