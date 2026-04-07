//! CDP Fetch domain — request interception, modification, and auth handling.

use super::{CdpResult, CdpTransport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ── Types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RequestPattern {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url_pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_stage: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderEntry {
    pub name: String,
    pub value: String,
}

// ── Events ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestPausedEvent {
    pub request_id: String,
    pub request: Value,
    pub frame_id: String,
    pub resource_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_error_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_status_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_headers: Option<Vec<HeaderEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthRequiredEvent {
    pub request_id: String,
    pub auth_challenge: Value,
}

// ── Methods ────────────────────────────────────────────────────────

pub async fn enable(
    transport: &dyn CdpTransport,
    patterns: Option<Vec<RequestPattern>>,
    handle_auth_requests: Option<bool>,
) -> CdpResult<()> {
    let mut params = json!({});
    if let Some(p) = patterns {
        params["patterns"] = serde_json::to_value(&p)?;
    }
    if let Some(h) = handle_auth_requests {
        params["handleAuthRequests"] = json!(h);
    }
    transport.send("Fetch.enable", params).await?;
    Ok(())
}

pub async fn disable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("Fetch.disable", json!({})).await?;
    Ok(())
}

pub async fn continue_request(
    transport: &dyn CdpTransport,
    request_id: &str,
    url: Option<&str>,
    method: Option<&str>,
    post_data: Option<&str>,
    headers: Option<Vec<HeaderEntry>>,
    intercept_response: Option<bool>,
) -> CdpResult<()> {
    let mut params = json!({ "requestId": request_id });
    if let Some(u) = url {
        params["url"] = json!(u);
    }
    if let Some(m) = method {
        params["method"] = json!(m);
    }
    if let Some(pd) = post_data {
        params["postData"] = json!(pd);
    }
    if let Some(h) = headers {
        params["headers"] = serde_json::to_value(&h)?;
    }
    if let Some(ir) = intercept_response {
        params["interceptResponse"] = json!(ir);
    }
    transport.send("Fetch.continueRequest", params).await?;
    Ok(())
}

pub async fn continue_response(
    transport: &dyn CdpTransport,
    request_id: &str,
    response_code: Option<i32>,
    response_phrase: Option<&str>,
    response_headers: Option<Vec<HeaderEntry>>,
    body: Option<&str>,
) -> CdpResult<()> {
    let mut params = json!({ "requestId": request_id });
    if let Some(c) = response_code {
        params["responseCode"] = json!(c);
    }
    if let Some(p) = response_phrase {
        params["responsePhrase"] = json!(p);
    }
    if let Some(h) = response_headers {
        params["responseHeaders"] = serde_json::to_value(&h)?;
    }
    if let Some(b) = body {
        params["body"] = json!(b);
    }
    transport.send("Fetch.continueResponse", params).await?;
    Ok(())
}

pub async fn fulfill_request(
    transport: &dyn CdpTransport,
    request_id: &str,
    response_code: i32,
    response_headers: Option<Vec<HeaderEntry>>,
    body: Option<&str>,
    response_phrase: Option<&str>,
) -> CdpResult<()> {
    let mut params = json!({
        "requestId": request_id,
        "responseCode": response_code,
    });
    if let Some(h) = response_headers {
        params["responseHeaders"] = serde_json::to_value(&h)?;
    }
    if let Some(b) = body {
        params["body"] = json!(b);
    }
    if let Some(p) = response_phrase {
        params["responsePhrase"] = json!(p);
    }
    transport.send("Fetch.fulfillRequest", params).await?;
    Ok(())
}

pub async fn fail_request(
    transport: &dyn CdpTransport,
    request_id: &str,
    error_reason: &str,
) -> CdpResult<()> {
    transport
        .send(
            "Fetch.failRequest",
            json!({
                "requestId": request_id,
                "errorReason": error_reason,
            }),
        )
        .await?;
    Ok(())
}

pub async fn get_response_body(
    transport: &dyn CdpTransport,
    request_id: &str,
) -> CdpResult<(String, bool)> {
    let raw = transport
        .send(
            "Fetch.getResponseBody",
            json!({ "requestId": request_id }),
        )
        .await?;
    let body = raw["body"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let base64_encoded = raw["base64Encoded"].as_bool().unwrap_or(false);
    Ok((body, base64_encoded))
}

pub async fn continue_with_auth(
    transport: &dyn CdpTransport,
    request_id: &str,
    response: &str,
    username: Option<&str>,
    password: Option<&str>,
) -> CdpResult<()> {
    let mut auth_response = json!({ "response": response });
    if let Some(u) = username {
        auth_response["username"] = json!(u);
    }
    if let Some(p) = password {
        auth_response["password"] = json!(p);
    }
    transport
        .send(
            "Fetch.continueWithAuth",
            json!({
                "requestId": request_id,
                "authChallengeResponse": auth_response,
            }),
        )
        .await?;
    Ok(())
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdp::MockTransport;
    use serde_json::json;

    #[tokio::test]
    async fn test_enable_with_patterns() {
        let mock = MockTransport::new();
        mock.expect("Fetch.enable", json!({})).await;

        let patterns = vec![
            RequestPattern {
                url_pattern: Some("*.js".into()),
                resource_type: Some("Script".into()),
                request_stage: Some("Request".into()),
            },
            RequestPattern {
                url_pattern: Some("*.css".into()),
                resource_type: None,
                request_stage: Some("Response".into()),
            },
        ];

        enable(&mock, Some(patterns), Some(true)).await.unwrap();
        mock.assert_called_once("Fetch.enable").await;

        let params = mock.call_params("Fetch.enable", 0).await.unwrap();
        assert_eq!(params["patterns"][0]["urlPattern"], "*.js");
        assert_eq!(params["patterns"][0]["resourceType"], "Script");
        assert_eq!(params["patterns"][1]["urlPattern"], "*.css");
        assert_eq!(params["handleAuthRequests"], true);
    }

    #[tokio::test]
    async fn test_disable() {
        let mock = MockTransport::new();
        mock.expect("Fetch.disable", json!({})).await;

        disable(&mock).await.unwrap();
        mock.assert_called_once("Fetch.disable").await;
    }

    #[tokio::test]
    async fn test_continue_request() {
        let mock = MockTransport::new();
        mock.expect("Fetch.continueRequest", json!({})).await;

        continue_request(
            &mock,
            "req-1",
            Some("https://modified.com"),
            Some("POST"),
            None,
            None,
            Some(true),
        )
        .await
        .unwrap();

        let params = mock.call_params("Fetch.continueRequest", 0).await.unwrap();
        assert_eq!(params["requestId"], "req-1");
        assert_eq!(params["url"], "https://modified.com");
        assert_eq!(params["method"], "POST");
        assert_eq!(params["interceptResponse"], true);
        assert!(params.get("postData").is_none());
    }

    #[tokio::test]
    async fn test_fulfill_request() {
        let mock = MockTransport::new();
        mock.expect("Fetch.fulfillRequest", json!({})).await;

        let headers = vec![
            HeaderEntry {
                name: "Content-Type".into(),
                value: "application/json".into(),
            },
            HeaderEntry {
                name: "X-Custom".into(),
                value: "test".into(),
            },
        ];

        fulfill_request(&mock, "req-2", 200, Some(headers), Some("{\"ok\":true}"), Some("OK"))
            .await
            .unwrap();

        let params = mock.call_params("Fetch.fulfillRequest", 0).await.unwrap();
        assert_eq!(params["requestId"], "req-2");
        assert_eq!(params["responseCode"], 200);
        assert_eq!(params["responseHeaders"][0]["name"], "Content-Type");
        assert_eq!(params["responseHeaders"][1]["name"], "X-Custom");
        assert_eq!(params["body"], "{\"ok\":true}");
        assert_eq!(params["responsePhrase"], "OK");
    }

    #[tokio::test]
    async fn test_fail_request() {
        let mock = MockTransport::new();
        mock.expect("Fetch.failRequest", json!({})).await;

        fail_request(&mock, "req-3", "AccessDenied").await.unwrap();

        let params = mock.call_params("Fetch.failRequest", 0).await.unwrap();
        assert_eq!(params["requestId"], "req-3");
        assert_eq!(params["errorReason"], "AccessDenied");
    }

    #[tokio::test]
    async fn test_get_response_body() {
        let mock = MockTransport::new();
        mock.expect(
            "Fetch.getResponseBody",
            json!({ "body": "aGVsbG8=", "base64Encoded": true }),
        )
        .await;

        let (body, encoded) = get_response_body(&mock, "req-4").await.unwrap();
        assert_eq!(body, "aGVsbG8=");
        assert!(encoded);
    }

    #[tokio::test]
    async fn test_continue_response() {
        let mock = MockTransport::new();
        mock.expect("Fetch.continueResponse", json!({})).await;

        let headers = vec![HeaderEntry {
            name: "X-Injected".into(),
            value: "yes".into(),
        }];

        continue_response(&mock, "req-5", Some(201), Some("Created"), Some(headers), None)
            .await
            .unwrap();

        let params = mock.call_params("Fetch.continueResponse", 0).await.unwrap();
        assert_eq!(params["requestId"], "req-5");
        assert_eq!(params["responseCode"], 201);
        assert_eq!(params["responsePhrase"], "Created");
        assert_eq!(params["responseHeaders"][0]["name"], "X-Injected");
        assert!(params.get("body").is_none());
    }

    #[tokio::test]
    async fn test_continue_with_auth() {
        let mock = MockTransport::new();
        mock.expect("Fetch.continueWithAuth", json!({})).await;

        continue_with_auth(&mock, "req-6", "ProvideCredentials", Some("user"), Some("pass"))
            .await
            .unwrap();

        let params = mock.call_params("Fetch.continueWithAuth", 0).await.unwrap();
        assert_eq!(params["requestId"], "req-6");
        assert_eq!(params["authChallengeResponse"]["response"], "ProvideCredentials");
        assert_eq!(params["authChallengeResponse"]["username"], "user");
        assert_eq!(params["authChallengeResponse"]["password"], "pass");
    }
}
