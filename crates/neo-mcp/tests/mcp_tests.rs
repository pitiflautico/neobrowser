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

    assert_eq!(
        result.get("url").and_then(|v| v.as_str()),
        Some("https://example.com")
    );
    assert_eq!(
        result.get("title").and_then(|v| v.as_str()),
        Some("Mock Page")
    );
    assert!(
        result.get("wom").is_some(),
        "wom should be present by default"
    );
}

#[test]
fn test_interact_click() {
    let mut state = init_state();
    // Navigate first so the mock has state
    let _ = tools::call_tool("browse", json!({"url": "https://example.com"}), &mut state);

    let result = tools::call_tool(
        "interact",
        json!({"action": "click", "target": "Submit"}),
        &mut state,
    )
    .expect("click failed");

    // MockBrowserEngine returns ClickResult::NoEffect by default
    assert_eq!(result.as_str(), Some("NoEffect"));
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

    assert_eq!(result.get("ok").and_then(|v| v.as_bool()), Some(true));
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
fn test_browse_without_extract() {
    let mut state = init_state();
    let result = tools::call_tool(
        "browse",
        json!({"url": "https://example.com", "extract": false}),
        &mut state,
    )
    .expect("browse failed");

    assert!(
        result.get("wom").unwrap().is_null(),
        "wom should be null when extract=false"
    );
}
