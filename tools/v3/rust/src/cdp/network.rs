//! CDP Network domain — cookies, headers, throttling, interception.

use super::{CdpResult, CdpTransport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

// ── Types ──────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub expires: f64,
    pub size: i32,
    pub http_only: bool,
    pub secure: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub same_site: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub same_party: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_scheme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_port: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partition_key: Option<Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CookieParam {
    pub name: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secure: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub same_site: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub same_party: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_scheme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_port: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partition_key: Option<Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Headers(pub HashMap<String, String>);

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub url: String,
    pub method: String,
    pub headers: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_post_data: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mixed_content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_priority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referrer_policy: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    pub url: String,
    pub status: i32,
    pub status_text: String,
    pub headers: Value,
    pub mime_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_headers: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_reused: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_ip_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_port: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_disk_cache: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_service_worker: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoded_data_length: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_state: Option<String>,
}

// ── Events ─────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RequestWillBeSentEvent {
    pub request_id: String,
    pub loader_id: String,
    pub document_url: String,
    pub request: Request,
    pub timestamp: f64,
    pub wall_time: f64,
    #[serde(rename = "type")]
    pub resource_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect_response: Option<Response>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ResponseReceivedEvent {
    pub request_id: String,
    pub loader_id: String,
    pub timestamp: f64,
    #[serde(rename = "type")]
    pub resource_type: String,
    pub response: Response,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LoadingFinishedEvent {
    pub request_id: String,
    pub timestamp: f64,
    pub encoded_data_length: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LoadingFailedEvent {
    pub request_id: String,
    pub timestamp: f64,
    #[serde(rename = "type")]
    pub resource_type: String,
    pub error_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canceled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NetworkConditions {
    pub offline: bool,
    pub latency: f64,
    pub download_throughput: f64,
    pub upload_throughput: f64,
}

impl NetworkConditions {
    pub fn offline() -> Self {
        Self { offline: true, latency: 0.0, download_throughput: -1.0, upload_throughput: -1.0 }
    }
    pub fn slow_3g() -> Self {
        Self { offline: false, latency: 2000.0, download_throughput: 50000.0, upload_throughput: 50000.0 }
    }
    pub fn fast_3g() -> Self {
        Self { offline: false, latency: 563.0, download_throughput: 180000.0, upload_throughput: 84375.0 }
    }
    pub fn regular_4g() -> Self {
        Self { offline: false, latency: 20.0, download_throughput: 4000000.0, upload_throughput: 3000000.0 }
    }
    pub fn wifi() -> Self {
        Self { offline: false, latency: 2.0, download_throughput: 30000000.0, upload_throughput: 15000000.0 }
    }
    pub fn no_throttle() -> Self {
        Self { offline: false, latency: 0.0, download_throughput: -1.0, upload_throughput: -1.0 }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SearchMatch {
    pub line_number: f64,
    pub line_content: String,
}

// ── Methods ────────────────────────────────────────────────────────

pub async fn enable(
    transport: &dyn CdpTransport,
    max_total_buffer_size: Option<i64>,
    max_resource_buffer_size: Option<i64>,
    max_post_data_size: Option<i64>,
) -> CdpResult<()> {
    let mut params = json!({});
    if let Some(v) = max_total_buffer_size {
        params["maxTotalBufferSize"] = json!(v);
    }
    if let Some(v) = max_resource_buffer_size {
        params["maxResourceBufferSize"] = json!(v);
    }
    if let Some(v) = max_post_data_size {
        params["maxPostDataSize"] = json!(v);
    }
    transport.send("Network.enable", params).await?;
    Ok(())
}

pub async fn disable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("Network.disable", json!({})).await?;
    Ok(())
}

pub async fn get_all_cookies(transport: &dyn CdpTransport) -> CdpResult<Vec<Cookie>> {
    let raw = transport.send("Network.getAllCookies", json!({})).await?;
    let cookies: Vec<Cookie> = serde_json::from_value(raw["cookies"].clone())?;
    Ok(cookies)
}

pub async fn get_cookies(
    transport: &dyn CdpTransport,
    urls: Option<&[&str]>,
) -> CdpResult<Vec<Cookie>> {
    let mut params = json!({});
    if let Some(urls) = urls {
        params["urls"] = json!(urls);
    }
    let raw = transport.send("Network.getCookies", params).await?;
    let cookies: Vec<Cookie> = serde_json::from_value(raw["cookies"].clone())?;
    Ok(cookies)
}

pub async fn set_cookies(
    transport: &dyn CdpTransport,
    cookies: Vec<CookieParam>,
) -> CdpResult<()> {
    let params = json!({ "cookies": cookies });
    transport.send("Network.setCookies", params).await?;
    Ok(())
}

pub async fn delete_cookies(
    transport: &dyn CdpTransport,
    name: &str,
    url: Option<&str>,
    domain: Option<&str>,
    path: Option<&str>,
) -> CdpResult<()> {
    let mut params = json!({ "name": name });
    if let Some(v) = url {
        params["url"] = json!(v);
    }
    if let Some(v) = domain {
        params["domain"] = json!(v);
    }
    if let Some(v) = path {
        params["path"] = json!(v);
    }
    transport.send("Network.deleteCookies", params).await?;
    Ok(())
}

pub async fn clear_browser_cookies(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("Network.clearBrowserCookies", json!({})).await?;
    Ok(())
}

pub async fn clear_browser_cache(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("Network.clearBrowserCache", json!({})).await?;
    Ok(())
}

pub async fn set_extra_http_headers(
    transport: &dyn CdpTransport,
    headers: HashMap<String, String>,
) -> CdpResult<()> {
    let params = json!({ "headers": headers });
    transport.send("Network.setExtraHTTPHeaders", params).await?;
    Ok(())
}

pub async fn set_user_agent_override(
    transport: &dyn CdpTransport,
    user_agent: &str,
    accept_language: Option<&str>,
    platform: Option<&str>,
) -> CdpResult<()> {
    let mut params = json!({ "userAgent": user_agent });
    if let Some(v) = accept_language {
        params["acceptLanguage"] = json!(v);
    }
    if let Some(v) = platform {
        params["platform"] = json!(v);
    }
    transport.send("Network.setUserAgentOverride", params).await?;
    Ok(())
}

pub async fn set_blocked_urls(
    transport: &dyn CdpTransport,
    urls: &[&str],
) -> CdpResult<()> {
    let params = json!({ "urls": urls });
    transport.send("Network.setBlockedURLs", params).await?;
    Ok(())
}

pub async fn emulate_network_conditions(
    transport: &dyn CdpTransport,
    conditions: NetworkConditions,
) -> CdpResult<()> {
    let params = json!({
        "offline": conditions.offline,
        "latency": conditions.latency,
        "downloadThroughput": conditions.download_throughput,
        "uploadThroughput": conditions.upload_throughput,
    });
    transport.send("Network.emulateNetworkConditions", params).await?;
    Ok(())
}

pub async fn set_cache_disabled(
    transport: &dyn CdpTransport,
    cache_disabled: bool,
) -> CdpResult<()> {
    let params = json!({ "cacheDisabled": cache_disabled });
    transport.send("Network.setCacheDisabled", params).await?;
    Ok(())
}

pub async fn get_response_body(
    transport: &dyn CdpTransport,
    request_id: &str,
) -> CdpResult<(String, bool)> {
    let params = json!({ "requestId": request_id });
    let raw = transport.send("Network.getResponseBody", params).await?;
    let body = raw["body"].as_str().unwrap_or("").to_string();
    let base64_encoded = raw["base64Encoded"].as_bool().unwrap_or(false);
    Ok((body, base64_encoded))
}

pub async fn get_request_post_data(
    transport: &dyn CdpTransport,
    request_id: &str,
) -> CdpResult<String> {
    let params = json!({ "requestId": request_id });
    let raw = transport.send("Network.getRequestPostData", params).await?;
    let post_data = raw["postData"].as_str().unwrap_or("").to_string();
    Ok(post_data)
}

pub async fn search_in_response_body(
    transport: &dyn CdpTransport,
    request_id: &str,
    query: &str,
    case_sensitive: Option<bool>,
    is_regex: Option<bool>,
) -> CdpResult<Vec<SearchMatch>> {
    let mut params = json!({
        "requestId": request_id,
        "query": query,
    });
    if let Some(v) = case_sensitive {
        params["caseSensitive"] = json!(v);
    }
    if let Some(v) = is_regex {
        params["isRegex"] = json!(v);
    }
    let raw = transport.send("Network.searchInResponseBody", params).await?;
    let matches: Vec<SearchMatch> = serde_json::from_value(raw["result"].clone())?;
    Ok(matches)
}

pub async fn set_request_interception(
    transport: &dyn CdpTransport,
    patterns: Vec<Value>,
) -> CdpResult<()> {
    let params = json!({ "patterns": patterns });
    transport.send("Network.setRequestInterception", params).await?;
    Ok(())
}

pub async fn get_certificate(
    transport: &dyn CdpTransport,
    origin: &str,
) -> CdpResult<Vec<String>> {
    let params = json!({ "origin": origin });
    let raw = transport.send("Network.getCertificate", params).await?;
    let table_names: Vec<String> = serde_json::from_value(raw["tableNames"].clone())?;
    Ok(table_names)
}

pub async fn set_bypass_service_worker(
    transport: &dyn CdpTransport,
    bypass: bool,
) -> CdpResult<()> {
    let params = json!({ "bypass": bypass });
    transport.send("Network.setBypassServiceWorker", params).await?;
    Ok(())
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdp::MockTransport;

    #[tokio::test]
    async fn test_enable() {
        let mock = MockTransport::new();
        mock.expect("Network.enable", json!({})).await;

        enable(&mock, Some(100_000_000), None, Some(65536)).await.unwrap();

        let params = mock.call_params("Network.enable", 0).await.unwrap();
        assert_eq!(params["maxTotalBufferSize"], 100_000_000);
        assert!(params.get("maxResourceBufferSize").is_none()
            || params["maxResourceBufferSize"].is_null());
        assert_eq!(params["maxPostDataSize"], 65536);
    }

    #[tokio::test]
    async fn test_get_all_cookies() {
        let mock = MockTransport::new();
        mock.expect("Network.getAllCookies", json!({
            "cookies": [
                {
                    "name": "sid",
                    "value": "abc123",
                    "domain": ".example.com",
                    "path": "/",
                    "expires": 1700000000.0,
                    "size": 9,
                    "httpOnly": true,
                    "secure": true,
                    "sameSite": "Lax"
                }
            ]
        })).await;

        let cookies = get_all_cookies(&mock).await.unwrap();
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "sid");
        assert_eq!(cookies[0].value, "abc123");
        assert_eq!(cookies[0].domain, ".example.com");
        assert!(cookies[0].http_only);
        assert!(cookies[0].secure);
        assert_eq!(cookies[0].same_site.as_deref(), Some("Lax"));
    }

    #[tokio::test]
    async fn test_get_cookies_with_urls() {
        let mock = MockTransport::new();
        mock.expect("Network.getCookies", json!({
            "cookies": [
                {
                    "name": "token",
                    "value": "xyz",
                    "domain": "api.example.com",
                    "path": "/v1",
                    "expires": -1.0,
                    "size": 8,
                    "httpOnly": false,
                    "secure": true
                }
            ]
        })).await;

        let urls = &["https://api.example.com/v1"];
        let cookies = get_cookies(&mock, Some(urls)).await.unwrap();
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "token");

        let params = mock.call_params("Network.getCookies", 0).await.unwrap();
        assert_eq!(params["urls"][0], "https://api.example.com/v1");
    }

    #[tokio::test]
    async fn test_set_cookies() {
        let mock = MockTransport::new();
        mock.expect("Network.setCookies", json!({})).await;

        let cookie = CookieParam {
            name: "session".into(),
            value: "s3cr3t".into(),
            url: Some("https://example.com".into()),
            domain: None,
            path: Some("/".into()),
            secure: Some(true),
            http_only: Some(true),
            same_site: Some("Strict".into()),
            expires: Some(1700000000.0),
            priority: None,
            same_party: None,
            source_scheme: None,
            source_port: None,
            partition_key: None,
        };
        set_cookies(&mock, vec![cookie]).await.unwrap();

        let params = mock.call_params("Network.setCookies", 0).await.unwrap();
        let c = &params["cookies"][0];
        assert_eq!(c["name"], "session");
        assert_eq!(c["value"], "s3cr3t");
        assert_eq!(c["url"], "https://example.com");
        assert_eq!(c["secure"], true);
        assert_eq!(c["httpOnly"], true);
        assert_eq!(c["sameSite"], "Strict");
        // Optional None fields should be absent
        assert!(c.get("domain").is_none() || c["domain"].is_null());
        assert!(c.get("priority").is_none() || c["priority"].is_null());
    }

    #[tokio::test]
    async fn test_delete_cookies() {
        let mock = MockTransport::new();
        mock.expect("Network.deleteCookies", json!({})).await;

        delete_cookies(
            &mock,
            "session",
            Some("https://example.com"),
            Some(".example.com"),
            Some("/app"),
        )
        .await
        .unwrap();

        let params = mock.call_params("Network.deleteCookies", 0).await.unwrap();
        assert_eq!(params["name"], "session");
        assert_eq!(params["url"], "https://example.com");
        assert_eq!(params["domain"], ".example.com");
        assert_eq!(params["path"], "/app");
    }

    #[tokio::test]
    async fn test_clear_browser_cookies() {
        let mock = MockTransport::new();
        mock.expect("Network.clearBrowserCookies", json!({})).await;

        clear_browser_cookies(&mock).await.unwrap();
        mock.assert_called_once("Network.clearBrowserCookies").await;
    }

    #[tokio::test]
    async fn test_set_extra_http_headers() {
        let mock = MockTransport::new();
        mock.expect("Network.setExtraHTTPHeaders", json!({})).await;

        let mut headers = HashMap::new();
        headers.insert("X-Custom".into(), "value1".into());
        headers.insert("Authorization".into(), "Bearer tok".into());
        set_extra_http_headers(&mock, headers).await.unwrap();

        let params = mock.call_params("Network.setExtraHTTPHeaders", 0).await.unwrap();
        assert_eq!(params["headers"]["X-Custom"], "value1");
        assert_eq!(params["headers"]["Authorization"], "Bearer tok");
    }

    #[tokio::test]
    async fn test_set_user_agent_override() {
        let mock = MockTransport::new();
        mock.expect("Network.setUserAgentOverride", json!({})).await;

        set_user_agent_override(
            &mock,
            "Mozilla/5.0 Custom",
            Some("en-US"),
            Some("Linux"),
        )
        .await
        .unwrap();

        let params = mock.call_params("Network.setUserAgentOverride", 0).await.unwrap();
        assert_eq!(params["userAgent"], "Mozilla/5.0 Custom");
        assert_eq!(params["acceptLanguage"], "en-US");
        assert_eq!(params["platform"], "Linux");
    }

    #[tokio::test]
    async fn test_set_blocked_urls() {
        let mock = MockTransport::new();
        mock.expect("Network.setBlockedURLs", json!({})).await;

        let urls = &["*.ads.com/*", "tracker.io/*"];
        set_blocked_urls(&mock, urls).await.unwrap();

        let params = mock.call_params("Network.setBlockedURLs", 0).await.unwrap();
        assert_eq!(params["urls"][0], "*.ads.com/*");
        assert_eq!(params["urls"][1], "tracker.io/*");
    }

    #[tokio::test]
    async fn test_emulate_network_conditions_offline() {
        let mock = MockTransport::new();
        mock.expect("Network.emulateNetworkConditions", json!({})).await;

        emulate_network_conditions(&mock, NetworkConditions::offline()).await.unwrap();

        let params = mock.call_params("Network.emulateNetworkConditions", 0).await.unwrap();
        assert_eq!(params["offline"], true);
        assert_eq!(params["latency"], 0.0);
        assert_eq!(params["downloadThroughput"], -1.0);
        assert_eq!(params["uploadThroughput"], -1.0);
    }

    #[tokio::test]
    async fn test_emulate_network_conditions_3g() {
        let mock = MockTransport::new();
        mock.expect("Network.emulateNetworkConditions", json!({})).await;

        emulate_network_conditions(&mock, NetworkConditions::slow_3g()).await.unwrap();

        let params = mock.call_params("Network.emulateNetworkConditions", 0).await.unwrap();
        assert_eq!(params["offline"], false);
        assert_eq!(params["latency"], 2000.0);
        assert_eq!(params["downloadThroughput"], 50000.0);
        assert_eq!(params["uploadThroughput"], 50000.0);
    }

    #[tokio::test]
    async fn test_network_conditions_presets() {
        let fast3g = NetworkConditions::fast_3g();
        assert!(!fast3g.offline);
        assert_eq!(fast3g.latency, 563.0);
        assert_eq!(fast3g.download_throughput, 180000.0);
        assert_eq!(fast3g.upload_throughput, 84375.0);

        let r4g = NetworkConditions::regular_4g();
        assert_eq!(r4g.latency, 20.0);
        assert_eq!(r4g.download_throughput, 4000000.0);

        let wifi = NetworkConditions::wifi();
        assert_eq!(wifi.latency, 2.0);
        assert_eq!(wifi.download_throughput, 30000000.0);
        assert_eq!(wifi.upload_throughput, 15000000.0);

        let no = NetworkConditions::no_throttle();
        assert!(!no.offline);
        assert_eq!(no.latency, 0.0);
        assert_eq!(no.download_throughput, -1.0);
        assert_eq!(no.upload_throughput, -1.0);
    }

    #[tokio::test]
    async fn test_set_cache_disabled() {
        let mock = MockTransport::new();
        mock.expect("Network.setCacheDisabled", json!({})).await;

        set_cache_disabled(&mock, true).await.unwrap();

        let params = mock.call_params("Network.setCacheDisabled", 0).await.unwrap();
        assert_eq!(params["cacheDisabled"], true);
    }

    #[tokio::test]
    async fn test_get_response_body() {
        let mock = MockTransport::new();
        mock.expect("Network.getResponseBody", json!({
            "body": "<html>hello</html>",
            "base64Encoded": false
        })).await;

        let (body, is_b64) = get_response_body(&mock, "req-1").await.unwrap();
        assert_eq!(body, "<html>hello</html>");
        assert!(!is_b64);

        let params = mock.call_params("Network.getResponseBody", 0).await.unwrap();
        assert_eq!(params["requestId"], "req-1");
    }

    #[tokio::test]
    async fn test_search_in_response_body() {
        let mock = MockTransport::new();
        mock.expect("Network.searchInResponseBody", json!({
            "result": [
                { "lineNumber": 5.0, "lineContent": "  var token = 'abc';" },
                { "lineNumber": 12.0, "lineContent": "  sendToken(token);" }
            ]
        })).await;

        let matches = search_in_response_body(
            &mock,
            "req-42",
            "token",
            Some(true),
            Some(false),
        )
        .await
        .unwrap();

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].line_number, 5.0);
        assert!(matches[0].line_content.contains("token"));
        assert_eq!(matches[1].line_number, 12.0);

        let params = mock.call_params("Network.searchInResponseBody", 0).await.unwrap();
        assert_eq!(params["requestId"], "req-42");
        assert_eq!(params["query"], "token");
        assert_eq!(params["caseSensitive"], true);
        assert_eq!(params["isRegex"], false);
    }

    #[tokio::test]
    async fn test_disable() {
        let mock = MockTransport::new();
        mock.expect("Network.disable", json!({})).await;

        disable(&mock).await.unwrap();
        mock.assert_called_once("Network.disable").await;
    }

    #[tokio::test]
    async fn test_set_bypass_service_worker() {
        let mock = MockTransport::new();
        mock.expect("Network.setBypassServiceWorker", json!({})).await;

        set_bypass_service_worker(&mock, true).await.unwrap();

        let params = mock.call_params("Network.setBypassServiceWorker", 0).await.unwrap();
        assert_eq!(params["bypass"], true);
    }

    #[tokio::test]
    async fn test_get_request_post_data() {
        let mock = MockTransport::new();
        mock.expect("Network.getRequestPostData", json!({
            "postData": "{\"username\":\"admin\"}"
        })).await;

        let data = get_request_post_data(&mock, "req-99").await.unwrap();
        assert_eq!(data, "{\"username\":\"admin\"}");

        let params = mock.call_params("Network.getRequestPostData", 0).await.unwrap();
        assert_eq!(params["requestId"], "req-99");
    }
}
