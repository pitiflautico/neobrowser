use neo_http::cache::DiskCache;
use neo_http::{CacheDecision, HttpCache, HttpRequest, RequestContext, RequestKind};
use neo_types::HttpResponse;
use std::collections::HashMap;

fn make_request(url: &str) -> HttpRequest {
    HttpRequest {
        method: "GET".into(),
        url: url.into(),
        headers: HashMap::new(),
        body: None,
        context: RequestContext {
            kind: RequestKind::Navigation,
            initiator: "test".into(),
            referrer: None,
            frame_id: None,
            top_level_url: None,
        },
        timeout_ms: 5000,
    }
}

fn make_response(url: &str, max_age: u64) -> HttpResponse {
    let mut headers = HashMap::new();
    headers.insert("cache-control".into(), format!("max-age={max_age}"));
    headers.insert("etag".into(), r#""abc123""#.into());
    HttpResponse {
        status: 200,
        headers,
        body: "<html>cached</html>".into(),
        url: url.into(),
        duration_ms: 50,
    }
}

#[test]
fn test_store_and_fresh_lookup() {
    let dir = tempfile::tempdir().unwrap();
    let cache = DiskCache::new(dir.path().to_str().unwrap()).unwrap();
    let url = "https://example.com/page";
    let req = make_request(url);
    let resp = make_response(url, 3600);

    cache.store(&req, &resp);
    match cache.lookup(&req) {
        CacheDecision::Fresh(r) => {
            assert_eq!(r.status, 200);
            assert!(r.body.contains("cached"));
        }
        other => panic!("expected Fresh, got: {other:?}"),
    }
    assert!(cache.is_fresh(url));
}

#[test]
fn test_miss_on_unknown_url() {
    let dir = tempfile::tempdir().unwrap();
    let cache = DiskCache::new(dir.path().to_str().unwrap()).unwrap();
    let req = make_request("https://unknown.com/nothing");
    assert!(matches!(cache.lookup(&req), CacheDecision::Miss));
    assert!(!cache.is_fresh("https://unknown.com/nothing"));
}
