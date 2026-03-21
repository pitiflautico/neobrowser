use neo_http::mock::MockHttpClient;
use neo_http::{HttpClient, HttpError, HttpRequest, RequestContext, RequestKind};
use neo_types::HttpResponse;
use std::collections::HashMap;

fn nav_request(url: &str) -> HttpRequest {
    HttpRequest {
        method: "GET".into(),
        url: url.into(),
        headers: HashMap::new(),
        body: None,
        context: RequestContext {
            kind: RequestKind::Navigation,
            initiator: "user".into(),
            referrer: None,
            frame_id: None,
            top_level_url: None,
        },
        timeout_ms: 5000,
    }
}

#[test]
fn test_mock_returns_configured_response() {
    let client = MockHttpClient::new();
    client.when_url("example.com").returns(HttpResponse {
        status: 200,
        headers: HashMap::new(),
        body: "<html>hello</html>".into(),
        url: "https://example.com/".into(),
        duration_ms: 10,
    });

    let req = nav_request("https://example.com/");
    let resp = client.request(&req).unwrap();
    assert_eq!(resp.status, 200);
    assert!(resp.body.contains("hello"));

    let recorded = client.requests();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].url, "https://example.com/");
}

#[test]
fn test_mock_no_rule_returns_error() {
    let client = MockHttpClient::new();
    let req = nav_request("https://unknown.com/");
    let result = client.request(&req);
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), HttpError::Network(_)));
}
