//! HTTP header parsing utilities.

use std::collections::HashMap;

/// Parse JSON headers string into HashMap.
pub fn parse_headers(json: &str) -> HashMap<String, String> {
    if json.is_empty() {
        return HashMap::new();
    }
    serde_json::from_str(json).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_headers_empty() {
        let h = parse_headers("");
        assert!(h.is_empty());
    }

    #[test]
    fn parse_headers_valid_json() {
        let h = parse_headers(r#"{"Content-Type":"application/json","X-Custom":"value"}"#);
        assert_eq!(h.get("Content-Type").unwrap(), "application/json");
        assert_eq!(h.get("X-Custom").unwrap(), "value");
    }

    #[test]
    fn parse_headers_invalid_json_returns_empty() {
        let h = parse_headers("not json");
        assert!(h.is_empty());
    }
}
