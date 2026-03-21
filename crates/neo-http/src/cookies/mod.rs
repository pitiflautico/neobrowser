//! Cookie store trait implementations.
//!
//! Provides [`SqliteCookieStore`] backed by SQLite for persistent
//! cookie storage with SameSite context awareness.

pub mod chrome;
mod sqlite;

pub use chrome::ChromeCookieImporter;
pub use sqlite::SqliteCookieStore;

/// Check if a cookie domain matches a request URL's host.
pub(crate) fn domain_matches(cookie_domain: &str, host: &str) -> bool {
    let cd = cookie_domain.trim_start_matches('.');
    host == cd || host.ends_with(&format!(".{cd}"))
}

/// Check if a cookie path matches a request path.
pub(crate) fn path_matches(cookie_path: &str, req_path: &str) -> bool {
    if cookie_path == "/" {
        return true;
    }
    req_path == cookie_path || req_path.starts_with(&format!("{cookie_path}/"))
}

/// Check if two URLs share the same registrable domain (simplified).
pub(crate) fn is_same_site(url: &str, top_level_url: &str) -> bool {
    let h1 = extract_host(url);
    let h2 = extract_host(top_level_url);
    let d1 = registrable_domain(&h1);
    let d2 = registrable_domain(&h2);
    d1 == d2
}

/// Extract host from a URL string.
pub(crate) fn extract_host(url: &str) -> String {
    url::Url::parse(url)
        .map(|u| u.host_str().unwrap_or("").to_string())
        .unwrap_or_default()
}

/// Simplified registrable domain: last two segments.
fn registrable_domain(host: &str) -> String {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() >= 2 {
        parts[parts.len() - 2..].join(".")
    } else {
        host.to_string()
    }
}

/// Current unix timestamp in seconds.
pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Parse a Set-Cookie header string into a Cookie struct.
pub(crate) fn parse_set_cookie(header: &str, default_domain: &str) -> neo_types::Cookie {
    let mut parts = header.split(';');
    let first = parts.next().unwrap_or("");
    let (name, value) = first.split_once('=').unwrap_or((first, ""));

    let mut cookie = neo_types::Cookie {
        name: name.trim().to_string(),
        value: value.trim().to_string(),
        domain: default_domain.to_string(),
        path: "/".to_string(),
        expires: None,
        http_only: false,
        secure: false,
        same_site: None,
    };

    for attr in parts {
        let attr = attr.trim();
        let (key, val) = attr.split_once('=').unwrap_or((attr, ""));
        match key.trim().to_lowercase().as_str() {
            "domain" => cookie.domain = val.trim().trim_start_matches('.').to_string(),
            "path" => cookie.path = val.trim().to_string(),
            "max-age" => {
                if let Ok(secs) = val.trim().parse::<i64>() {
                    cookie.expires = Some(now_secs() + secs);
                }
            }
            "httponly" => cookie.http_only = true,
            "secure" => cookie.secure = true,
            "samesite" => cookie.same_site = Some(val.trim().to_string()),
            _ => {}
        }
    }
    cookie
}
