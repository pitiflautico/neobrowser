//! `import_cookies` tool — import cookies from a Chrome profile.

use serde_json::Value;

use crate::McpError;

use super::ToolDef;

/// Tool definition for `tools/list`.
pub(crate) fn definition() -> ToolDef {
    ToolDef {
        name: "import_cookies",
        description: "Import cookies from a Chrome profile into NeoRender's cookie store",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "chrome_profile": {
                    "type": "string",
                    "description": "Chrome profile name (default: \"Profile 24\")",
                    "default": "Profile 24"
                },
                "domain": {
                    "type": "string",
                    "description": "Optional domain filter (e.g. \"chatgpt.com\")"
                }
            }
        }),
    }
}

/// Execute the `import_cookies` tool.
pub fn call(args: Value) -> Result<Value, McpError> {
    let profile = args
        .get("chrome_profile")
        .and_then(|v| v.as_str())
        .unwrap_or("Profile 24");

    let domain = args.get("domain").and_then(|v| v.as_str());

    let importer = neo_http::ChromeCookieImporter::new(profile, domain);
    let cookies = importer
        .import()
        .map_err(|e| McpError::InvalidParams(format!("Chrome import failed: {e}")))?;

    if cookies.is_empty() {
        return Ok(serde_json::json!({
            "count": 0,
            "domains": [],
            "message": "No cookies found"
        }));
    }

    // Collect domain summary.
    let mut domain_counts: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::new();
    for c in &cookies {
        *domain_counts.entry(&c.domain).or_default() += 1;
    }
    let domains: Vec<Value> = domain_counts
        .iter()
        .map(|(d, n)| {
            serde_json::json!({
                "domain": d,
                "count": n
            })
        })
        .collect();

    // Import into the default cookie store.
    let store = neo_http::SqliteCookieStore::default_store()
        .map_err(|e| McpError::InvalidParams(format!("Cookie store error: {e}")))?;

    use neo_http::CookieStore;
    store.import(&cookies);

    let total = cookies.len();
    Ok(serde_json::json!({
        "count": total,
        "domains": domains,
        "message": format!("Imported {total} cookies from Chrome \"{profile}\"")
    }))
}
