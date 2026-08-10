//! Optional LLM fallback for semantic `find` (Layer 3).
//!
//! Cost-safe by design: this only ever calls the Anthropic API when the user has
//! set `ANTHROPIC_API_KEY`. With no key it is a no-op (returns `None`), so the
//! heuristic `find` remains the zero-cost default and nothing is spent unless the
//! user opts in with their own key. Model overridable via `ANTHROPIC_MODEL`
//! (default: a Haiku tier).
//!
//! Security: the model is only asked to *choose among* backendNodeIds we already
//! extracted; the caller validates the returned id against that set, so a prompt
//! injection in page text cannot make us click an element that wasn't in the snapshot.

use std::time::Duration;

use serde_json::json;

const DEFAULT_MODEL: &str = "claude-haiku-4-5-20251001";

/// True if an Anthropic key is configured (i.e. the LLM fallback is available).
pub fn available() -> bool {
    std::env::var("ANTHROPIC_API_KEY")
        .map(|k| !k.is_empty())
        .unwrap_or(false)
}

/// Ask the LLM to pick the backendNodeId matching `intent` from a `snapshot`
/// (one element per line: `role | "name" | backendNodeId`). Returns the chosen id,
/// or `None` (no key, network/parse failure, or no match). Never panics, never
/// blocks the heuristic path.
pub async fn find_by_intent(snapshot: &str, intent: &str) -> Option<i64> {
    let key = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())?;
    let model = std::env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());

    let prompt = format!(
        "You locate a UI element in a web page.\n\
         Accessibility snapshot (one element per line as: role | \"name\" | backendNodeId):\n\
         {snapshot}\n\n\
         Intent: {intent}\n\n\
         Return ONLY a JSON object {{\"backendNodeId\": <number>}} for the single best match, \
         or {{\"backendNodeId\": null}} if none fits. No prose."
    );

    let body = json!({
        "model": model,
        "max_tokens": 100,
        "messages": [{ "role": "user", "content": prompt }],
    });

    let resp = reqwest::Client::new()
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .timeout(Duration::from_secs(20))
        .json(&body)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: serde_json::Value = resp.json().await.ok()?;
    // content[0].text holds the model's reply.
    let text = v
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|b| b.get("text"))
        .and_then(|t| t.as_str())?;
    parse_backend_node_id(text)
}

/// Extract `backendNodeId` from the model's reply, tolerant of surrounding prose.
fn parse_backend_node_id(text: &str) -> Option<i64> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    let obj: serde_json::Value = serde_json::from_str(&text[start..=end]).ok()?;
    obj.get("backendNodeId").and_then(|v| v.as_i64())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_json() {
        assert_eq!(parse_backend_node_id(r#"{"backendNodeId": 42}"#), Some(42));
    }

    #[test]
    fn parses_json_with_prose() {
        assert_eq!(
            parse_backend_node_id("Sure! Here you go:\n{\"backendNodeId\": 7}\nHope that helps"),
            Some(7)
        );
    }

    #[test]
    fn null_and_garbage_return_none() {
        assert_eq!(parse_backend_node_id(r#"{"backendNodeId": null}"#), None);
        assert_eq!(parse_backend_node_id("no json here"), None);
        assert_eq!(parse_backend_node_id(""), None);
    }

    #[test]
    fn available_reflects_env() {
        let prev = std::env::var("ANTHROPIC_API_KEY").ok();
        std::env::remove_var("ANTHROPIC_API_KEY");
        assert!(!available());
        std::env::set_var("ANTHROPIC_API_KEY", "sk-test");
        assert!(available());
        match prev {
            Some(v) => std::env::set_var("ANTHROPIC_API_KEY", v),
            None => std::env::remove_var("ANTHROPIC_API_KEY"),
        }
    }
}
