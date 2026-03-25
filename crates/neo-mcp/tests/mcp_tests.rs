//! Integration tests for neo-mcp tool handlers.

use neo_mcp::mock::mock_with_page;
use neo_mcp::state::McpState;
use neo_mcp::tools;
use serde_json::json;

fn init_state() -> McpState {
    let engine = mock_with_page("https://example.com", "Example");
    let mut state = McpState::new(Box::new(engine));
    state.initialized = true;
    state
}

#[test]
fn test_tools_list() {
    let list = tools::list_tools();
    let tools_arr = list.get("tools").expect("missing tools key");
    let tools_arr = tools_arr.as_array().expect("tools not array");

    assert!(tools_arr.len() >= 4, "expected at least 4 tools");

    let names: Vec<&str> = tools_arr
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
        .collect();

    assert!(names.contains(&"browse"));
    assert!(names.contains(&"interact"));
    assert!(names.contains(&"extract"));
    assert!(names.contains(&"trace"));

    // Every tool must have an inputSchema
    for tool in tools_arr {
        assert!(tool.get("inputSchema").is_some(), "missing inputSchema");
    }
}

#[test]
fn test_browse_calls_navigate() {
    let mut state = init_state();
    let result = tools::call_tool("browse", json!({"url": "https://example.com"}), &mut state)
        .expect("browse failed");

    // browse returns compact text view
    let text = result.as_str().expect("browse should return string");
    assert!(text.contains("https://example.com"), "should contain URL");
}

#[test]
fn test_interact_click() {
    let mut state = init_state();
    let _ = tools::call_tool("browse", json!({"url": "https://example.com"}), &mut state);

    let result = tools::call_tool(
        "interact",
        json!({"action": "click", "target": "Submit"}),
        &mut state,
    )
    .expect("click failed");

    // interact returns compact text with action result + page view
    let text = result.as_str().expect("interact should return string");
    assert!(text.contains("[click]"), "should contain action tag");
}

#[test]
fn test_interact_type() {
    let mut state = init_state();
    let result = tools::call_tool(
        "interact",
        json!({"action": "type", "target": "email", "text": "user@test.com"}),
        &mut state,
    )
    .expect("type failed");

    let text = result.as_str().expect("interact should return string");
    assert!(text.contains("[type]"), "should contain action tag");
}

#[test]
fn test_extract_wom() {
    let mut state = init_state();
    let result =
        tools::call_tool("extract", json!({"kind": "wom"}), &mut state).expect("extract failed");

    // WomDocument has nodes array
    let nodes = result
        .get("nodes")
        .and_then(|v| v.as_array())
        .expect("missing nodes");
    assert_eq!(nodes.len(), 2, "mock has 2 nodes");
    assert_eq!(
        result.get("page_type").and_then(|v| v.as_str()),
        Some("article")
    );
}

#[test]
fn test_trace_summary() {
    let mut state = init_state();
    // Do an action so summary has content
    let _ = tools::call_tool("browse", json!({"url": "https://example.com"}), &mut state);

    let result =
        tools::call_tool("trace", json!({"kind": "summary"}), &mut state).expect("trace failed");

    assert!(
        result.get("total_actions").is_some(),
        "summary should have total_actions"
    );
    assert!(result.get("state").is_some(), "summary should have state");
}

#[test]
fn test_unknown_tool_returns_error() {
    let mut state = init_state();
    let err = tools::call_tool("nonexistent", json!({}), &mut state);
    assert!(err.is_err());
}

#[test]
fn test_wait_for_text() {
    let mut state = init_state();
    let result = tools::call_tool(
        "wait",
        json!({"text": "Hello World", "timeout_ms": 1000}),
        &mut state,
    )
    .expect("wait for text failed");

    assert_eq!(result.get("found").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        result.get("text").and_then(|v| v.as_str()),
        Some("Hello World")
    );
}

#[test]
fn test_wait_requires_selector_or_text() {
    let mut state = init_state();
    let err = tools::call_tool("wait", json!({"timeout_ms": 1000}), &mut state);
    assert!(err.is_err());
}

#[test]
fn test_navigate_back_forward() {
    let mut state = init_state();
    let _ = tools::call_tool("navigate", json!({"url": "https://page1.com"}), &mut state);
    let _ = tools::call_tool("navigate", json!({"url": "https://page2.com"}), &mut state);

    let result = tools::call_tool("navigate", json!({"action": "back"}), &mut state)
        .expect("back failed");
    let text = result.as_str().expect("navigate should return string");
    assert!(text.contains("url:"), "should contain url");

    let result = tools::call_tool("navigate", json!({"action": "forward"}), &mut state)
        .expect("forward failed");
    assert!(result.as_str().is_some());
}

#[test]
fn test_navigate_requires_url_or_action() {
    let mut state = init_state();
    let err = tools::call_tool("navigate", json!({}), &mut state);
    assert!(err.is_err());
}

#[test]
fn test_page_tool() {
    let mut state = init_state();
    // Navigate so there's a page loaded.
    let _ = tools::call_tool("browse", json!({"url": "https://example.com"}), &mut state);

    let result =
        tools::call_tool("page", json!({}), &mut state).expect("page failed");

    assert!(result.get("url").is_some());
    assert!(result.get("page_type").is_some());
    assert!(result.get("summary").is_some());
    assert!(result.get("node_count").is_some());
    assert!(result.get("page_id").is_some());
    // full WOM not included by default
    assert!(result.get("wom").is_none(), "wom absent without full=true");
}

#[test]
fn test_page_tool_full() {
    let mut state = init_state();
    let _ = tools::call_tool("browse", json!({"url": "https://example.com"}), &mut state);

    let result =
        tools::call_tool("page", json!({"full": true}), &mut state).expect("page failed");

    assert!(result.get("wom").is_some(), "wom present with full=true");
}

#[test]
fn test_tools_list_includes_new_tools() {
    let list = tools::list_tools();
    let tools_arr = list.get("tools").unwrap().as_array().unwrap();
    let names: Vec<&str> = tools_arr
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
        .collect();

    assert!(names.contains(&"navigate"), "missing navigate tool");
    assert!(names.contains(&"page"), "missing page tool");
}

#[test]
fn test_browse_without_extract() {
    let mut state = init_state();
    // browse no longer accepts extract param — always returns compact view
    let result = tools::call_tool(
        "browse",
        json!({"url": "https://example.com"}),
        &mut state,
    )
    .expect("browse failed");

    let text = result.as_str().expect("browse should return string");
    assert!(text.contains("url:"), "should contain url");
}

// -- Pipeline tests --

#[test]
fn test_pipeline_basic() {
    let mut state = init_state();
    let result = tools::call_tool(
        "pipeline",
        json!({
            "steps": [
                {"action": "browse", "url": "https://example.com"},
                {"action": "click", "target": "Submit"},
                {"action": "extract", "kind": "text"},
            ]
        }),
        &mut state,
    )
    .expect("pipeline failed");

    assert_eq!(result.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(result.get("steps_completed").and_then(|v| v.as_u64()), Some(3));
    assert_eq!(result.get("steps_total").and_then(|v| v.as_u64()), Some(3));
    assert!(result.get("total_ms").is_some());

    let results = result.get("results").and_then(|v| v.as_array()).unwrap();
    assert_eq!(results.len(), 3);
    for r in results {
        assert_eq!(r.get("ok").and_then(|v| v.as_bool()), Some(true));
        assert!(r.get("ms").is_some());
    }
}

#[test]
fn test_pipeline_stops_on_error() {
    let mut state = init_state();
    // "type" without target should fail
    let result = tools::call_tool(
        "pipeline",
        json!({
            "steps": [
                {"action": "browse", "url": "https://example.com"},
                {"action": "type"},
                {"action": "click", "target": "Submit"},
            ]
        }),
        &mut state,
    )
    .expect("pipeline call itself should not fail");

    assert_eq!(result.get("ok").and_then(|v| v.as_bool()), Some(false));
    // Should have stopped at step 1 (the type step that failed)
    let results = result.get("results").and_then(|v| v.as_array()).unwrap();
    assert_eq!(results.len(), 2); // browse ok + type error
    assert_eq!(results[0].get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(results[1].get("ok").and_then(|v| v.as_bool()), Some(false));
}

#[test]
fn test_pipeline_continue_on_error() {
    let mut state = init_state();
    let result = tools::call_tool(
        "pipeline",
        json!({
            "steps": [
                {"action": "browse", "url": "https://example.com"},
                {"action": "type", "continue_on_error": true},
                {"action": "click", "target": "Submit"},
            ]
        }),
        &mut state,
    )
    .expect("pipeline call itself should not fail");

    // Pipeline had error but continued
    assert_eq!(result.get("ok").and_then(|v| v.as_bool()), Some(false));
    let results = result.get("results").and_then(|v| v.as_array()).unwrap();
    assert_eq!(results.len(), 3); // all 3 ran
    assert_eq!(results[0].get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(results[1].get("ok").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(results[2].get("ok").and_then(|v| v.as_bool()), Some(true));
}

#[test]
fn test_pipeline_empty_steps_error() {
    let mut state = init_state();
    let err = tools::call_tool("pipeline", json!({"steps": []}), &mut state);
    assert!(err.is_err());
}

#[test]
fn test_pipeline_in_tools_list() {
    let list = tools::list_tools();
    let tools_arr = list.get("tools").unwrap().as_array().unwrap();
    let names: Vec<&str> = tools_arr
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
        .collect();
    assert!(names.contains(&"pipeline"), "missing pipeline tool");
}

// -- Enhanced fill_form tests --

#[test]
fn test_fill_form_enhanced_returns_details() {
    let mut state = init_state();
    let result = tools::call_tool(
        "interact",
        json!({
            "action": "fill_form",
            "fields": {"email": "user@test.com", "password": "secret"}
        }),
        &mut state,
    )
    .expect("fill_form failed");

    // fill_form returns compact text view
    let text = result.as_str().expect("fill_form should return string");
    assert!(text.contains("[fill_form]"), "should contain action tag");
}

// -- analyze_forms tests --

#[test]
fn test_analyze_forms() {
    let mut state = init_state();
    let result = tools::call_tool(
        "interact",
        json!({"action": "analyze_forms"}),
        &mut state,
    )
    .expect("analyze_forms failed");

    assert_eq!(result.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert!(result.get("forms").is_some());
    assert!(result.get("total_forms").is_some());
    assert!(result.get("total_fields").is_some());
}

// ═══════════════════════════════════════════════════════════════
// Devtools tool tests
// ═══════════════════════════════════════════════════════════════

fn init_devtools_state(panel: &str, response_json: &str) -> McpState {
    let mut engine = neo_mcp::mock::mock_with_page("https://example.com", "Example");
    engine.eval_result = response_json.to_string();
    let mut state = McpState::new(Box::new(engine));
    state.initialized = true;
    state
}

#[test]
fn devtools_listed_in_tools() {
    let list = tools::list_tools();
    let tools_arr = list.get("tools").unwrap().as_array().unwrap();
    let names: Vec<&str> = tools_arr
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
        .collect();
    assert!(names.contains(&"devtools"), "devtools tool should be listed");
}

#[test]
fn devtools_network_summary() {
    let json_resp = r#"{"total":3,"byMethod":{"GET":2,"POST":1},"byStatus":{"200":2,"404":1},"errors":1,"totalBytes":4096,"avgDuration":150}"#;
    let mut state = init_devtools_state("network", json_resp);

    let result = tools::call_tool("devtools", json!({"panel": "network"}), &mut state)
        .expect("devtools network failed");

    assert_eq!(result.get("total").and_then(|v| v.as_u64()), Some(3));
    assert!(result.get("byMethod").is_some());
    assert_eq!(result.get("errors").and_then(|v| v.as_u64()), Some(1));
}

#[test]
fn devtools_network_filter() {
    let json_resp = r#"[{"id":1,"url":"https://api.example.com/data","method":"GET","status":200}]"#;
    let mut state = init_devtools_state("network", json_resp);

    let result = tools::call_tool(
        "devtools",
        json!({"panel": "network", "filter": "api.example"}),
        &mut state,
    )
    .expect("devtools network filter failed");

    let arr = result.as_array().expect("should be array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0].get("url").and_then(|v| v.as_str()), Some("https://api.example.com/data"));
}

#[test]
fn devtools_network_failed() {
    let json_resp = r#"[{"id":2,"url":"https://example.com/missing","method":"GET","status":404,"error":null}]"#;
    let mut state = init_devtools_state("network_failed", json_resp);

    let result = tools::call_tool("devtools", json!({"panel": "network_failed"}), &mut state)
        .expect("devtools network_failed failed");

    let arr = result.as_array().expect("should be array");
    assert_eq!(arr.len(), 1);
}

#[test]
fn devtools_console_messages() {
    let json_resp = r#"[{"level":"log","message":"hello world","timestamp":1234567890,"stack":null},{"level":"error","message":"something broke","timestamp":1234567891,"stack":"Error\n at foo"}]"#;
    let mut state = init_devtools_state("console", json_resp);

    let result = tools::call_tool("devtools", json!({"panel": "console"}), &mut state)
        .expect("devtools console failed");

    let arr = result.as_array().expect("should be array");
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0].get("level").and_then(|v| v.as_str()), Some("log"));
    assert_eq!(arr[1].get("level").and_then(|v| v.as_str()), Some("error"));
}

#[test]
fn devtools_console_errors_only() {
    let json_resp = r#"[{"level":"error","message":"crash","timestamp":1234567891,"stack":null}]"#;
    let mut state = init_devtools_state("console_errors", json_resp);

    let result = tools::call_tool("devtools", json!({"panel": "console_errors"}), &mut state)
        .expect("devtools console_errors failed");

    let arr = result.as_array().expect("should be array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0].get("level").and_then(|v| v.as_str()), Some("error"));
}

#[test]
fn devtools_errors_summary() {
    let json_resp = r#"{"exceptions":2,"rejections":1,"lastException":"TypeError: x is not a function","lastRejection":"fetch failed"}"#;
    let mut state = init_devtools_state("errors", json_resp);

    let result = tools::call_tool("devtools", json!({"panel": "errors"}), &mut state)
        .expect("devtools errors failed");

    assert_eq!(result.get("exceptions").and_then(|v| v.as_u64()), Some(2));
    assert_eq!(result.get("rejections").and_then(|v| v.as_u64()), Some(1));
    assert_eq!(
        result.get("lastException").and_then(|v| v.as_str()),
        Some("TypeError: x is not a function")
    );
}

#[test]
fn devtools_errors_all() {
    let json_resp = r#"{"exceptions":[{"message":"ReferenceError","source":"app.js","line":42,"column":5,"stack":null,"timestamp":123}],"rejections":[{"message":"network error","stack":null,"timestamp":456}]}"#;
    let mut state = init_devtools_state("errors_all", json_resp);

    let result = tools::call_tool("devtools", json!({"panel": "errors_all"}), &mut state)
        .expect("devtools errors_all failed");

    assert!(result.get("exceptions").is_some());
    assert!(result.get("rejections").is_some());
}

#[test]
fn devtools_cookies() {
    let json_resp = r#"[{"name":"session_id","value":"abc123"},{"name":"theme","value":"dark"}]"#;
    let mut state = init_devtools_state("cookies", json_resp);

    let result = tools::call_tool("devtools", json!({"panel": "cookies"}), &mut state)
        .expect("devtools cookies failed");

    let arr = result.as_array().expect("should be array");
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0].get("name").and_then(|v| v.as_str()), Some("session_id"));
}

#[test]
fn devtools_all_summary() {
    let json_resp = r#"{"network":{"total":5,"byMethod":{},"byStatus":{},"errors":0,"totalBytes":0,"avgDuration":0},"console":{"total":10,"byLevel":{"log":8,"error":2}},"errors":{"exceptions":0,"rejections":0,"lastException":null,"lastRejection":null},"cookies":{"count":1,"names":["sid"]}}"#;
    let mut state = init_devtools_state("all", json_resp);

    let result = tools::call_tool("devtools", json!({"panel": "all"}), &mut state)
        .expect("devtools all failed");

    assert!(result.get("network").is_some());
    assert!(result.get("console").is_some());
    assert!(result.get("errors").is_some());
    assert!(result.get("cookies").is_some());
}

#[test]
fn devtools_invalid_panel() {
    let mut state = init_devtools_state("whatever", "{}");
    let err = tools::call_tool("devtools", json!({"panel": "invalid_panel"}), &mut state);
    assert!(err.is_err(), "unknown panel should return error");
}

#[test]
fn devtools_missing_panel() {
    let mut state = init_devtools_state("whatever", "{}");
    let err = tools::call_tool("devtools", json!({}), &mut state);
    assert!(err.is_err(), "missing panel should return error");
}
