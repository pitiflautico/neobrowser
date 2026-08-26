//! Network request interception: block URLs by pattern.
//!
//! Uses CDP's `Network.setBlockedURLs`, which is the simplest reliable way to
//! prevent trackers, ads, or specific endpoints from loading. Patterns support
//! `*` wildcards (e.g. `*tracker*`, `*.doubleclick.net`).

use serde_json::json;

use crate::cdp::CdpClient;

/// Block network requests whose URL matches any of `patterns`.
///
/// Patterns are Chrome's URL patterns: `*` matches any sequence of characters.
/// The block stays active until `unblock_urls` is called or the tab closes.
pub async fn block_urls(client: &CdpClient, patterns: &[&str]) -> Result<String, String> {
    let blocked: Vec<serde_json::Value> = patterns.iter().map(|p| json!(p)).collect();
    client
        .send("Network.setBlockedURLs", json!({ "urls": blocked }))
        .await
        .map_err(|e| format!("setBlockedURLs failed: {e}"))?;
    Ok(format!(
        "blocking {} pattern(s): {}",
        patterns.len(),
        patterns.join(", ")
    ))
}

/// Remove all URL blocks set by `block_urls`.
pub async fn unblock_urls(client: &CdpClient) -> Result<String, String> {
    client
        .send("Network.setBlockedURLs", json!({ "urls": [] }))
        .await
        .map_err(|e| format!("setBlockedURLs failed: {e}"))?;
    Ok("all URL blocks removed".to_string())
}

/// Block common trackers and ads. Convenience preset.
pub async fn block_trackers(client: &CdpClient) -> Result<String, String> {
    block_urls(
        client,
        &[
            "*google-analytics.com*",
            "*googletagmanager.com*",
            "*doubleclick.net*",
            "*facebook.net*",
            "*facebook.com/tr*",
            "*connect.facebook.net*",
            "*hotjar.com*",
            "*mixpanel.com*",
            "*segment.io*",
            "*amplitude.com*",
        ],
    )
    .await
}
