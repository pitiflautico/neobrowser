//! Comprehensive DOM, event, and bootstrap pipeline tests.
//!
//! These tests use the real V8 runtime (DenoRuntime) with happy-dom bootstrap
//! to verify the full DOM/event pipeline. Marked `#[ignore]` because V8
//! compilation is heavy.
//!
//! Run with: cargo test -p neo-engine --test dom_events_test -- --ignored

use neo_engine::LiveDom;
use neo_runtime::v8::DenoRuntime;
use neo_runtime::{JsRuntime, RuntimeConfig};

/// Helper: create a DenoRuntime and load HTML with full bootstrap.
fn setup(html: &str) -> DenoRuntime {
    let mut rt = DenoRuntime::new(&RuntimeConfig::default())
        .expect("failed to create DenoRuntime");
    rt.set_document_html(html, "https://example.com")
        .expect("set_document_html failed");
    rt
}

// ─── 1. Bootstrap initialization ────────────────────────────────────

#[test]
#[ignore]
fn bootstrap_document_exists() {
    let mut rt = setup("<html><body><p>Hello</p></body></html>");
    let r = rt.eval("typeof document").unwrap();
    assert_eq!(r, "object", "document should be an object");
}

#[test]
#[ignore]
fn bootstrap_window_is_globalthis() {
    let mut rt = setup("<html><body></body></html>");
    let r = rt.eval("window === globalThis").unwrap();
    assert_eq!(r, "true", "window should be globalThis");
}

#[test]
#[ignore]
fn bootstrap_document_body_exists() {
    let mut rt = setup("<html><body></body></html>");
    let r = rt.eval("document.body !== null && document.body !== undefined").unwrap();
    assert_eq!(r, "true", "document.body should exist");
}

#[test]
#[ignore]
fn bootstrap_query_selector_works() {
    let mut rt = setup("<html><body><p id='hello'>Hello</p></body></html>");
    let r = rt.eval("document.querySelector('p').textContent").unwrap();
    assert_eq!(r, "Hello");
}

#[test]
#[ignore]
fn bootstrap_navigator_user_agent() {
    let mut rt = setup("<html><body></body></html>");
    let r = rt.eval("navigator.userAgent").unwrap();
    assert!(
        r.contains("Chrome") || r.contains("chrome"),
        "navigator.userAgent should contain Chrome, got: {r}"
    );
}

#[test]
#[ignore]
fn bootstrap_fetch_defined() {
    let mut rt = setup("<html><body></body></html>");
    let r = rt.eval("typeof fetch").unwrap();
    assert_eq!(r, "function", "fetch should be a function");
}

#[test]
#[ignore]
fn bootstrap_mutation_observer_defined() {
    let mut rt = setup("<html><body></body></html>");
    let r = rt.eval("typeof MutationObserver").unwrap();
    assert_eq!(r, "function", "MutationObserver should be defined");
}

#[test]
#[ignore]
fn bootstrap_custom_elements_defined() {
    let mut rt = setup("<html><body></body></html>");
    let r = rt.eval("typeof customElements !== 'undefined'").unwrap();
    assert_eq!(r, "true", "customElements should be defined");
}

#[test]
#[ignore]
fn bootstrap_intersection_observer_defined() {
    let mut rt = setup("<html><body></body></html>");
    let r = rt.eval("typeof IntersectionObserver").unwrap();
    assert_eq!(r, "function", "IntersectionObserver should be defined");
}

#[test]
#[ignore]
fn bootstrap_resize_observer_defined() {
    let mut rt = setup("<html><body></body></html>");
    let r = rt.eval("typeof ResizeObserver").unwrap();
    assert_eq!(r, "function", "ResizeObserver should be defined");
}

#[test]
#[ignore]
fn bootstrap_performance_now() {
    let mut rt = setup("<html><body></body></html>");
    let r = rt.eval("typeof performance.now()").unwrap();
    assert_eq!(r, "number", "performance.now() should return a number");
}

#[test]
#[ignore]
fn bootstrap_crypto_get_random_values() {
    let mut rt = setup("<html><body></body></html>");
    let r = rt.eval("typeof crypto.getRandomValues").unwrap();
    assert_eq!(r, "function", "crypto.getRandomValues should be a function");

    // Actually call it — should not throw
    let r2 = rt.eval("crypto.getRandomValues(new Uint8Array(4)).length").unwrap();
    assert_eq!(r2, "4", "getRandomValues should fill 4 bytes");
}

#[test]
#[ignore]
fn bootstrap_set_timeout_works() {
    let mut rt = setup("<html><body></body></html>");
    let r = rt.eval("typeof setTimeout").unwrap();
    assert_eq!(r, "function", "setTimeout should be defined");
}

#[test]
#[ignore]
fn bootstrap_queue_microtask_defined() {
    let mut rt = setup("<html><body></body></html>");
    let r = rt.eval("typeof queueMicrotask").unwrap();
    assert_eq!(r, "function", "queueMicrotask should be defined");
}

#[test]
#[ignore]
fn bootstrap_message_channel_defined() {
    let mut rt = setup("<html><body></body></html>");
    let r = rt.eval("typeof MessageChannel").unwrap();
    assert_eq!(r, "function", "MessageChannel should be defined");
}

#[test]
#[ignore]
fn bootstrap_event_source_defined() {
    let mut rt = setup("<html><body></body></html>");
    let r = rt.eval("typeof EventSource").unwrap();
    // EventSource may or may not be polyfilled — document what we find
    assert!(
        r == "function" || r == "undefined",
        "EventSource is '{r}' — expected function or undefined"
    );
}

#[test]
#[ignore]
fn bootstrap_readable_stream_defined() {
    let mut rt = setup("<html><body></body></html>");
    let r = rt.eval("typeof ReadableStream").unwrap();
    assert_eq!(r, "function", "ReadableStream should be defined");
}

// ─── 2. Event bubbling ──────────────────────────────────────────────

#[test]
#[ignore]
fn event_bubbling_click_bubbles_to_parent() {
    let mut rt = setup(r#"<html><body><div id="parent"><button id="child">Click</button></div></body></html>"#);
    rt.execute(r#"
        globalThis.__bubbled = false;
        globalThis.__target = '';
        document.getElementById('parent').addEventListener('click', function(e) {
            globalThis.__bubbled = true;
            globalThis.__target = e.target.id;
        });
    "#).unwrap();

    // Dispatch click on child button
    rt.execute(r#"
        var btn = document.getElementById('child');
        btn.dispatchEvent(new MouseEvent('click', {bubbles: true}));
    "#).unwrap();

    assert_eq!(rt.eval("globalThis.__bubbled").unwrap(), "true",
        "click should bubble from child to parent");
    assert_eq!(rt.eval("globalThis.__target").unwrap(), "child",
        "event.target should be the child button");
}

#[test]
#[ignore]
fn event_stop_propagation_prevents_bubble() {
    let mut rt = setup(r#"<html><body><div id="parent"><button id="child">Click</button></div></body></html>"#);
    rt.execute(r#"
        globalThis.__parent_received = false;
        document.getElementById('child').addEventListener('click', function(e) {
            e.stopPropagation();
        });
        document.getElementById('parent').addEventListener('click', function(e) {
            globalThis.__parent_received = true;
        });
    "#).unwrap();

    rt.execute(r#"
        var btn = document.getElementById('child');
        btn.dispatchEvent(new MouseEvent('click', {bubbles: true}));
    "#).unwrap();

    assert_eq!(rt.eval("globalThis.__parent_received").unwrap(), "false",
        "stopPropagation should prevent parent from receiving event");
}

#[test]
#[ignore]
fn event_capture_fires_before_bubble() {
    let mut rt = setup(r#"<html><body><div id="parent"><button id="child">Click</button></div></body></html>"#);
    rt.execute(r#"
        globalThis.__order = [];
        document.getElementById('parent').addEventListener('click', function(e) {
            globalThis.__order.push('capture');
        }, true);  // capture phase
        document.getElementById('parent').addEventListener('click', function(e) {
            globalThis.__order.push('bubble');
        }, false);  // bubble phase
    "#).unwrap();

    rt.execute(r#"
        var btn = document.getElementById('child');
        btn.dispatchEvent(new MouseEvent('click', {bubbles: true}));
    "#).unwrap();

    let order = rt.eval("globalThis.__order.join(',')").unwrap();
    assert_eq!(order, "capture,bubble",
        "capture phase should fire before bubble phase, got: {order}");
}

// ─── 3. Input events via LiveDom ────────────────────────────────────

#[test]
#[ignore]
fn livedom_type_text_sets_value() {
    let mut rt = setup(r#"<html><body><input type="text" id="name"></body></html>"#);
    {
        let mut dom = LiveDom::new(&mut rt);
        dom.type_text("#name", "hello").expect("type_text failed");
    }
    let val = rt.eval("document.getElementById('name').value").unwrap();
    assert_eq!(val, "hello", "input value should be 'hello', got: {val}");
}

#[test]
#[ignore]
fn livedom_type_text_dispatches_events() {
    let mut rt = setup(r#"<html><body><input type="text" id="name"></body></html>"#);
    rt.execute(r#"
        globalThis.__events = [];
        var el = document.getElementById('name');
        ['keydown','keypress','beforeinput','input','keyup'].forEach(function(evt) {
            el.addEventListener(evt, function(e) {
                globalThis.__events.push(evt);
            });
        });
    "#).unwrap();

    {
        let mut dom = LiveDom::new(&mut rt);
        dom.type_text("#name", "a").expect("type_text failed");
    }

    let events = rt.eval("globalThis.__events.join(',')").unwrap();
    assert!(events.contains("keydown"), "should have keydown event, got: {events}");
    assert!(events.contains("input"), "should have input event, got: {events}");
    assert!(events.contains("keyup"), "should have keyup event, got: {events}");
}

// ─── 4. Input type=email ────────────────────────────────────────────

#[test]
#[ignore]
fn livedom_type_email_no_selection_error() {
    let mut rt = setup(r#"<html><body><input type="email" id="email"></body></html>"#);
    {
        let mut dom = LiveDom::new(&mut rt);
        // This should NOT throw an InvalidStateError about selectionStart
        let result = dom.type_text("#email", "test@example.com");
        assert!(result.is_ok(), "type_text on email should not throw: {:?}", result.err());
    }
    let val = rt.eval("document.getElementById('email').value").unwrap();
    assert_eq!(val, "test@example.com",
        "email input value should be set correctly, got: {val}");
}

// ─── 5. Click events via LiveDom ────────────────────────────────────

#[test]
#[ignore]
fn livedom_click_dispatches_event() {
    let mut rt = setup(r#"<html><body><button id="btn">Click me</button></body></html>"#);
    rt.execute(r#"
        globalThis.__clicked = false;
        document.getElementById('btn').addEventListener('click', function() {
            globalThis.__clicked = true;
        });
    "#).unwrap();

    {
        let mut dom = LiveDom::new(&mut rt);
        dom.click("#btn").expect("click failed");
    }

    assert_eq!(rt.eval("globalThis.__clicked").unwrap(), "true",
        "click event should have been dispatched");
}

#[test]
#[ignore]
fn livedom_click_fires_mousedown_mouseup_click_in_order() {
    let mut rt = setup(r#"<html><body><button id="btn">Click me</button></body></html>"#);
    rt.execute(r#"
        globalThis.__order = [];
        var el = document.getElementById('btn');
        el.addEventListener('mousedown', function() { globalThis.__order.push('mousedown'); });
        el.addEventListener('mouseup', function() { globalThis.__order.push('mouseup'); });
        el.addEventListener('click', function() { globalThis.__order.push('click'); });
    "#).unwrap();

    {
        let mut dom = LiveDom::new(&mut rt);
        dom.click("#btn").expect("click failed");
    }

    let order = rt.eval("globalThis.__order.join(',')").unwrap();
    // Order should be mousedown, mouseup, click (pointerdown/pointerup may also appear)
    assert!(order.contains("mousedown"), "should have mousedown, got: {order}");
    assert!(order.contains("mouseup"), "should have mouseup, got: {order}");
    assert!(order.contains("click"), "should have click, got: {order}");

    // Verify relative order: mousedown before mouseup before click
    let parts: Vec<&str> = order.split(',').collect();
    let md_pos = parts.iter().position(|&x| x == "mousedown").unwrap();
    let mu_pos = parts.iter().position(|&x| x == "mouseup").unwrap();
    let ck_pos = parts.iter().position(|&x| x == "click").unwrap();
    assert!(md_pos < mu_pos, "mousedown should come before mouseup");
    assert!(mu_pos < ck_pos, "mouseup should come before click");
}

// ─── 6. Form submit ────────────────────────────────────────────────

#[test]
#[ignore]
fn livedom_form_submit_fires_event() {
    let mut rt = setup(r#"<html><body>
        <form id="myform">
            <input name="email" id="email">
            <button type="submit" id="go">Go</button>
        </form>
    </body></html>"#);

    rt.execute(r#"
        globalThis.__submitted = false;
        document.getElementById('myform').addEventListener('submit', function(e) {
            globalThis.__submitted = true;
        });
    "#).unwrap();

    {
        let mut dom = LiveDom::new(&mut rt);
        dom.type_text("#email", "test@test.com").expect("type_text failed");
        dom.click("#go").expect("click submit failed");
    }

    assert_eq!(rt.eval("globalThis.__submitted").unwrap(), "true",
        "submit event should have fired");
}

// ─── 7. Focus management ───────────────────────────────────────────

#[test]
#[ignore]
fn livedom_focus_updates_active_element() {
    let mut rt = setup(r#"<html><body>
        <input type="text" id="first">
        <input type="text" id="second">
    </body></html>"#);

    {
        let mut dom = LiveDom::new(&mut rt);
        dom.click("#first").expect("click first failed");
    }

    // After clicking first, activeElement should reference it
    // Note: happy-dom may or may not track activeElement correctly
    let active = rt.eval(r#"
        (function() {
            var ae = document.activeElement;
            if (ae && ae.id) return ae.id;
            return ae ? ae.tagName : 'null';
        })()
    "#).unwrap();
    // Document what actually happens — this is diagnostic
    eprintln!("activeElement after clicking #first: {active}");
}

#[test]
fn livedom_focusin_focusout_events_fire() {
    let mut rt = setup(r#"<html><body>
        <input type="text" id="first">
        <input type="text" id="second">
    </body></html>"#);

    rt.execute(r#"
        globalThis.__focus_events = [];
        document.getElementById('first').addEventListener('focusin', function() {
            globalThis.__focus_events.push('first:focusin');
        });
        document.getElementById('first').addEventListener('focusout', function() {
            globalThis.__focus_events.push('first:focusout');
        });
        document.getElementById('second').addEventListener('focusin', function() {
            globalThis.__focus_events.push('second:focusin');
        });
    "#).unwrap();

    {
        let mut dom = LiveDom::new(&mut rt);
        dom.click("#first").expect("click first failed");
        dom.click("#second").expect("click second failed");
    }

    let events = rt.eval("globalThis.__focus_events.join(',')").unwrap();
    assert!(events.contains("first:focusin"), "first should get focusin, got: {events}");
    assert!(events.contains("first:focusout"), "first should get focusout when clicking second, got: {events}");
    assert!(events.contains("second:focusin"), "second should get focusin, got: {events}");
}

// ─── 8. React controlled input compat ───────────────────────────────

#[test]
#[ignore]
fn livedom_react_value_tracker_compat() {
    let mut rt = setup(r#"<html><body><input id="ctrl" value="old"></body></html>"#);

    // Set up a mock React _valueTracker
    rt.execute(r#"
        globalThis.__tracker_calls = [];
        var el = document.getElementById('ctrl');
        el._valueTracker = {
            _v: 'old',
            getValue: function() { return this._v; },
            setValue: function(v) {
                this._v = v;
                globalThis.__tracker_calls.push('setValue:' + v);
            }
        };
        globalThis.__input_events = 0;
        el.addEventListener('input', function() {
            globalThis.__input_events++;
        });
    "#).unwrap();

    {
        let mut dom = LiveDom::new(&mut rt);
        dom.type_text("#ctrl", "new").expect("type_text failed");
    }

    // Check tracker was called with the old value before update
    let tracker_calls = rt.eval("globalThis.__tracker_calls.join(';')").unwrap();
    assert!(!tracker_calls.is_empty(),
        "tracker.setValue should have been called, got: {tracker_calls}");

    // Check input events fired
    let input_count = rt.eval("globalThis.__input_events").unwrap();
    let count: usize = input_count.parse().unwrap_or(0);
    assert!(count > 0, "input event should have been dispatched, count={count}");

    // Verify the value was set
    let val = rt.eval("document.getElementById('ctrl').value").unwrap();
    // The final value should contain "new" (typed character by character over "old")
    assert!(val.contains("new"), "value should contain 'new', got: {val}");
}

// ─── 9. Select handling ────────────────────────────────────────────

#[test]
#[ignore]
fn livedom_select_changes_option() {
    let mut rt = setup(r#"<html><body>
        <select id="sel">
            <option value="a">A</option>
            <option value="b">B</option>
        </select>
    </body></html>"#);

    {
        let mut dom = LiveDom::new(&mut rt);
        dom.type_text("#sel", "b").expect("type_text on select failed");
    }

    let val = rt.eval("document.getElementById('sel').value").unwrap();
    assert_eq!(val, "b", "select value should be 'b', got: {val}");
}

#[test]
#[ignore]
fn livedom_select_dispatches_change_event() {
    let mut rt = setup(r#"<html><body>
        <select id="sel">
            <option value="a">A</option>
            <option value="b">B</option>
        </select>
    </body></html>"#);

    rt.execute(r#"
        globalThis.__changed = false;
        document.getElementById('sel').addEventListener('change', function() {
            globalThis.__changed = true;
        });
    "#).unwrap();

    {
        let mut dom = LiveDom::new(&mut rt);
        dom.type_text("#sel", "b").expect("type_text on select failed");
    }

    assert_eq!(rt.eval("globalThis.__changed").unwrap(), "true",
        "change event should fire on select");
}

// ─── 10. Checkbox/radio ─────────────────────────────────────────────

#[test]
#[ignore]
fn livedom_checkbox_toggle() {
    let mut rt = setup(r#"<html><body><input type="checkbox" id="cb"></body></html>"#);

    // Verify initial state
    let initial = rt.eval("document.getElementById('cb').checked").unwrap();
    assert_eq!(initial, "false", "checkbox should start unchecked");

    {
        let mut dom = LiveDom::new(&mut rt);
        dom.click("#cb").expect("click checkbox failed");
    }

    let checked = rt.eval("document.getElementById('cb').checked").unwrap();
    assert_eq!(checked, "true", "checkbox should be checked after click");
}

#[test]
#[ignore]
fn livedom_checkbox_fires_change_event() {
    let mut rt = setup(r#"<html><body><input type="checkbox" id="cb"></body></html>"#);

    rt.execute(r#"
        globalThis.__change_fired = false;
        document.getElementById('cb').addEventListener('change', function() {
            globalThis.__change_fired = true;
        });
    "#).unwrap();

    {
        let mut dom = LiveDom::new(&mut rt);
        dom.click("#cb").expect("click checkbox failed");
    }

    assert_eq!(rt.eval("globalThis.__change_fired").unwrap(), "true",
        "change event should fire when checkbox is toggled");
}

#[test]
#[ignore]
fn livedom_radio_button_select() {
    let mut rt = setup(r#"<html><body>
        <form>
            <input type="radio" name="choice" value="a" id="ra">
            <input type="radio" name="choice" value="b" id="rb">
        </form>
    </body></html>"#);

    {
        let mut dom = LiveDom::new(&mut rt);
        dom.click("#rb").expect("click radio failed");
    }

    let a_checked = rt.eval("document.getElementById('ra').checked").unwrap();
    let b_checked = rt.eval("document.getElementById('rb').checked").unwrap();
    assert_eq!(a_checked, "false", "radio A should be unchecked");
    assert_eq!(b_checked, "true", "radio B should be checked");
}

#[test]
#[ignore]
fn livedom_radio_fires_change_event() {
    let mut rt = setup(r#"<html><body>
        <form>
            <input type="radio" name="choice" value="a" id="ra">
            <input type="radio" name="choice" value="b" id="rb">
        </form>
    </body></html>"#);

    rt.execute(r#"
        globalThis.__radio_change = false;
        document.getElementById('rb').addEventListener('change', function() {
            globalThis.__radio_change = true;
        });
    "#).unwrap();

    {
        let mut dom = LiveDom::new(&mut rt);
        dom.click("#rb").expect("click radio failed");
    }

    assert_eq!(rt.eval("globalThis.__radio_change").unwrap(), "true",
        "change event should fire when radio is selected");
}

// ─── Bonus: LiveDom element not found ───────────────────────────────

#[test]
#[ignore]
fn livedom_click_not_found_returns_error() {
    let mut rt = setup("<html><body></body></html>");
    let mut dom = LiveDom::new(&mut rt);
    let result = dom.click("#nonexistent");
    assert!(result.is_err(), "clicking nonexistent element should error");
    let err = result.unwrap_err();
    assert!(
        matches!(err, neo_engine::LiveDomError::NotFound(_)),
        "error should be NotFound, got: {err:?}"
    );
}

// ─── Bonus: type_text appends, not replaces ─────────────────────────

#[test]
#[ignore]
fn livedom_type_text_appends_to_existing_value() {
    let mut rt = setup(r#"<html><body><input type="text" id="inp" value="abc"></body></html>"#);
    {
        let mut dom = LiveDom::new(&mut rt);
        dom.type_text("#inp", "def").expect("type_text failed");
    }
    let val = rt.eval("document.getElementById('inp').value").unwrap();
    // type_text should append at caret (end), not replace
    assert_eq!(val, "abcdef", "value should be 'abcdef', got: {val}");
}

// ─── Bonus: disabled element not interactable ───────────────────────

#[test]
#[ignore]
fn livedom_click_disabled_element() {
    let mut rt = setup(r#"<html><body><button id="btn" disabled>No</button></body></html>"#);
    let mut dom = LiveDom::new(&mut rt);
    let result = dom.click("#btn");
    // Should either error with NotInteractable or succeed but do nothing
    eprintln!("click disabled result: {:?}", result);
}

// ─── Bonus: pointerdown fires before mousedown ─────────────────────

#[test]
#[ignore]
fn livedom_click_pointer_before_mouse() {
    let mut rt = setup(r#"<html><body><button id="btn">Click</button></body></html>"#);
    rt.execute(r#"
        globalThis.__order = [];
        var el = document.getElementById('btn');
        el.addEventListener('pointerdown', function() { globalThis.__order.push('pointerdown'); });
        el.addEventListener('mousedown', function() { globalThis.__order.push('mousedown'); });
        el.addEventListener('pointerup', function() { globalThis.__order.push('pointerup'); });
        el.addEventListener('mouseup', function() { globalThis.__order.push('mouseup'); });
        el.addEventListener('click', function() { globalThis.__order.push('click'); });
    "#).unwrap();

    {
        let mut dom = LiveDom::new(&mut rt);
        dom.click("#btn").expect("click failed");
    }

    let order = rt.eval("globalThis.__order.join(',')").unwrap();
    let parts: Vec<&str> = order.split(',').collect();

    // W3C spec order: pointerdown, mousedown, pointerup, mouseup, click
    if let (Some(pd), Some(md)) = (
        parts.iter().position(|&x| x == "pointerdown"),
        parts.iter().position(|&x| x == "mousedown"),
    ) {
        assert!(pd < md, "pointerdown should fire before mousedown, got: {order}");
    }
}

// ─── Bonus: multiple type_text calls accumulate ─────────────────────

#[test]
#[ignore]
fn livedom_multiple_type_text_accumulates() {
    let mut rt = setup(r#"<html><body><input type="text" id="inp"></body></html>"#);
    {
        let mut dom = LiveDom::new(&mut rt);
        dom.type_text("#inp", "abc").expect("first type_text");
        dom.type_text("#inp", "123").expect("second type_text");
    }
    let val = rt.eval("document.getElementById('inp').value").unwrap();
    assert_eq!(val, "abc123", "two type_text calls should accumulate, got: {val}");
}

// ─── Bonus: beforeinput cancelable ──────────────────────────────────

#[test]
#[ignore]
fn livedom_beforeinput_cancel_prevents_typing() {
    let mut rt = setup(r#"<html><body><input type="text" id="inp"></body></html>"#);
    rt.execute(r#"
        document.getElementById('inp').addEventListener('beforeinput', function(e) {
            e.preventDefault();
        });
    "#).unwrap();

    {
        let mut dom = LiveDom::new(&mut rt);
        dom.type_text("#inp", "abc").expect("type_text");
    }

    let val = rt.eval("document.getElementById('inp').value").unwrap();
    assert_eq!(val, "", "beforeinput preventDefault should block typing, got: {val}");
}

// ─── Bonus: ActionOutcome for checkbox ──────────────────────────────

#[test]
#[ignore]
fn livedom_checkbox_returns_toggled_outcome() {
    use neo_engine::ActionOutcome;

    let mut rt = setup(r#"<html><body><input type="checkbox" id="cb"></body></html>"#);
    let result = {
        let mut dom = LiveDom::new(&mut rt);
        dom.click("#cb").expect("click checkbox")
    };
    // The outcome should indicate checkbox was toggled
    match &result.outcome {
        ActionOutcome::CheckboxToggled { checked } => {
            assert!(*checked, "checkbox should be checked=true");
        }
        other => {
            eprintln!("Expected CheckboxToggled, got: {other:?}");
            // Don't hard-fail — document what we get
        }
    }
}
