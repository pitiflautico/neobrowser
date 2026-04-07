//! UnifiedCookieJar — single source of truth for all cookies.
//! SQLite-backed, syncs between HTTP Set-Cookie headers and JS document.cookie.

use rusqlite::Connection;
use std::sync::Mutex;

/// Shared handle for storing in OpState (Send + Sync required by deno_core).
pub type CookieJarHandle = std::sync::Arc<UnifiedCookieJar>;

pub struct UnifiedCookieJar {
    db: Mutex<Connection>,
}

impl UnifiedCookieJar {
    /// Create/open the SQLite cookie database.
    pub fn new() -> Result<Self, String> {
        let storage_dir = dirs::home_dir()
            .map(|h| h.join(".neobrowser").join("storage"))
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp/neobrowser-storage"));
        std::fs::create_dir_all(&storage_dir).ok();
        let db_path = storage_dir.join("cookies.db");
        let conn = Connection::open(&db_path).map_err(|e| format!("Cookie DB: {e}"))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cookies (
                domain TEXT NOT NULL,
                path TEXT NOT NULL DEFAULT '/',
                name TEXT NOT NULL,
                value TEXT NOT NULL,
                secure INTEGER DEFAULT 0,
                http_only INTEGER DEFAULT 0,
                same_site TEXT DEFAULT 'Lax',
                expires INTEGER,
                PRIMARY KEY (domain, path, name)
            );"
        ).map_err(|e| format!("Cookie table: {e}"))?;
        // Clean up expired cookies on open
        conn.execute(
            "DELETE FROM cookies WHERE expires IS NOT NULL AND expires < ?1",
            [now_unix()],
        ).ok();
        eprintln!("[COOKIEJAR] SQLite cookie jar ready at {:?}", db_path);
        Ok(Self { db: Mutex::new(conn) })
    }

    /// Store a cookie from an HTTP Set-Cookie header.
    /// Parses: `name=value; Domain=.example.com; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=3600`
    pub fn store_from_header(&self, url: &str, header: &str) {
        let parts: Vec<&str> = header.split(';').collect();
        if parts.is_empty() { return; }

        // Parse name=value from first part
        let kv: Vec<&str> = parts[0].splitn(2, '=').collect();
        if kv.len() != 2 { return; }
        let name = kv[0].trim();
        let value = kv[1].trim();
        if name.is_empty() { return; }

        // Derive default domain from URL
        let parsed_url = url::Url::parse(url).ok();
        let default_domain = parsed_url.as_ref()
            .and_then(|u| u.host_str())
            .unwrap_or("")
            .to_string();

        let mut domain = default_domain.clone();
        let mut path = "/".to_string();
        let mut secure = parsed_url.as_ref().map(|u| u.scheme() == "https").unwrap_or(false) as i32;
        let mut http_only: i32 = 0;
        let mut same_site = "Lax".to_string();
        let mut expires: Option<i64> = None;

        // Parse attributes
        for part in &parts[1..] {
            let p = part.trim();
            let p_lower = p.to_lowercase();
            if p_lower.starts_with("domain=") {
                domain = p[7..].trim().trim_start_matches('.').to_string();
            } else if p_lower.starts_with("path=") {
                path = p[5..].trim().to_string();
            } else if p_lower == "secure" {
                secure = 1;
            } else if p_lower == "httponly" {
                http_only = 1;
            } else if p_lower.starts_with("samesite=") {
                same_site = p[9..].trim().to_string();
            } else if p_lower.starts_with("max-age=") {
                if let Ok(secs) = p[8..].trim().parse::<i64>() {
                    if secs <= 0 {
                        // Delete cookie
                        self.delete_cookie(&domain, &path, name);
                        return;
                    }
                    expires = Some(now_unix() + secs);
                }
            } else if p_lower.starts_with("expires=") {
                // Parse HTTP date — simplified: if Max-Age was present it takes precedence (already handled above)
                if expires.is_none() {
                    if let Ok(ts) = parse_http_date(&p[8..].trim()) {
                        if ts < now_unix() {
                            self.delete_cookie(&domain, &path, name);
                            return;
                        }
                        expires = Some(ts);
                    }
                }
            }
        }

        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT OR REPLACE INTO cookies (domain, path, name, value, secure, http_only, same_site, expires) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![domain, path, name, value, secure, http_only, same_site, expires],
        ).ok();
    }

    /// Store a cookie from JS `document.cookie = "name=value; path=/; ..."`.
    pub fn store_from_js(&self, domain: &str, cookie_str: &str) {
        let parts: Vec<&str> = cookie_str.split(';').collect();
        if parts.is_empty() { return; }
        let kv: Vec<&str> = parts[0].splitn(2, '=').collect();
        if kv.len() != 2 { return; }
        let name = kv[0].trim();
        let value = kv[1].trim();
        if name.is_empty() { return; }

        let mut path = "/".to_string();
        let mut expires: Option<i64> = None;
        let mut same_site = "Lax".to_string();

        for part in &parts[1..] {
            let p = part.trim();
            let p_lower = p.to_lowercase();
            if p_lower.starts_with("path=") {
                path = p[5..].trim().to_string();
            } else if p_lower.starts_with("max-age=") {
                if let Ok(secs) = p[8..].trim().parse::<i64>() {
                    if secs <= 0 {
                        self.delete_cookie(domain, &path, name);
                        return;
                    }
                    expires = Some(now_unix() + secs);
                }
            } else if p_lower.starts_with("expires=") {
                if expires.is_none() {
                    if let Ok(ts) = parse_http_date(&p[8..].trim()) {
                        if ts < now_unix() {
                            self.delete_cookie(domain, &path, name);
                            return;
                        }
                        expires = Some(ts);
                    }
                }
            } else if p_lower.starts_with("samesite=") {
                same_site = p[9..].trim().to_string();
            }
        }

        // JS cannot set HttpOnly cookies
        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT OR REPLACE INTO cookies (domain, path, name, value, secure, http_only, same_site, expires) VALUES (?1, ?2, ?3, ?4, 0, 0, ?5, ?6)",
            rusqlite::params![domain, path, name, value, same_site, expires],
        ).ok();
    }

    /// Build the Cookie header value for an HTTP request to this URL.
    /// Matches domain (including parent domains) and path.
    /// Only includes Secure cookies for HTTPS URLs.
    pub fn cookie_header_for(&self, url: &str) -> Option<String> {
        let parsed = url::Url::parse(url).ok()?;
        let hostname = parsed.host_str()?;
        let req_path = parsed.path();
        let is_https = parsed.scheme() == "https";
        let now = now_unix();

        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare(
            "SELECT name, value, domain, path, secure, expires FROM cookies"
        ).ok()?;

        let mut pairs: Vec<String> = Vec::new();
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i32>(4)?,
                row.get::<_, Option<i64>>(5)?,
            ))
        }).ok()?;

        for row in rows.flatten() {
            let (name, value, domain, path, secure, expires) = row;
            // Check expiry
            if let Some(exp) = expires {
                if exp < now { continue; }
            }
            // Domain matching: hostname must equal domain or end with .domain
            if !domain_matches(hostname, &domain) { continue; }
            // Path matching
            if !req_path.starts_with(&path) { continue; }
            // Secure check
            if secure == 1 && !is_https { continue; }

            pairs.push(format!("{}={}", name, value));
        }

        if pairs.is_empty() { None } else { Some(pairs.join("; ")) }
    }

    /// Get document.cookie string for JS — excludes HttpOnly cookies.
    pub fn js_cookie_string(&self, domain: &str) -> String {
        let now = now_unix();
        let db = self.db.lock().unwrap();
        let mut stmt = match db.prepare(
            "SELECT name, value, domain, http_only, expires FROM cookies WHERE http_only = 0"
        ) {
            Ok(s) => s,
            Err(_) => return String::new(),
        };

        let mut pairs: Vec<String> = Vec::new();
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i32>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        });

        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (name, value, cookie_domain, _http_only, expires) = row;
                if let Some(exp) = expires {
                    if exp < now { continue; }
                }
                if !domain_matches(domain, &cookie_domain) { continue; }
                pairs.push(format!("{}={}", name, value));
            }
        }
        pairs.join("; ")
    }

    /// Load cookies from a JSON file (Chrome export format).
    /// Accepts: `[{name, value, domain, path, httpOnly, secure, expirationDate}]`
    /// or `{cookies: [...]}`.
    pub fn load_from_file(&self, path: &str) -> Result<usize, String> {
        let data = std::fs::read_to_string(path).map_err(|e| format!("{e}"))?;
        let parsed: serde_json::Value = serde_json::from_str(&data).map_err(|e| format!("{e}"))?;

        let cookies: Vec<serde_json::Value> = if let Some(arr) = parsed.as_array() {
            arr.clone()
        } else if let Some(arr) = parsed["cookies"].as_array() {
            arr.clone()
        } else {
            return Err("Expected JSON array or {cookies:[...]}".to_string());
        };

        let db = self.db.lock().unwrap();
        let mut count = 0;
        for c in cookies {
            let name = c["name"].as_str().unwrap_or_default();
            let value = c["value"].as_str().unwrap_or_default();
            let domain = c["domain"].as_str().unwrap_or_default().trim_start_matches('.');
            if name.is_empty() || domain.is_empty() { continue; }

            let path = c["path"].as_str().unwrap_or("/");
            let secure = c["secure"].as_bool().unwrap_or(false) as i32;
            let http_only = c["httpOnly"].as_bool().unwrap_or(false) as i32;
            let same_site = c["sameSite"].as_str().unwrap_or("Lax");
            let expires: Option<i64> = c["expirationDate"].as_f64()
                .or_else(|| c["expires"].as_f64())
                .map(|f| f as i64);

            db.execute(
                "INSERT OR REPLACE INTO cookies (domain, path, name, value, secure, http_only, same_site, expires) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![domain, path, name, value, secure, http_only, same_site, expires],
            ).ok();
            count += 1;
        }
        eprintln!("[COOKIEJAR] Loaded {} cookies from {}", count, path);
        Ok(count)
    }

    /// Clear all cookies for a domain.
    pub fn clear_domain(&self, domain: &str) {
        let db = self.db.lock().unwrap();
        db.execute("DELETE FROM cookies WHERE domain = ?1", [domain]).ok();
    }

    /// Get all cookies as domain → "name=val; name2=val2" map (compat with ghost::CookieJar).
    pub fn all_headers(&self) -> std::collections::HashMap<String, String> {
        let db = self.db.lock().unwrap();
        let mut stmt = match db.prepare("SELECT domain, name, value FROM cookies") {
            Ok(s) => s,
            Err(_) => return std::collections::HashMap::new(),
        };
        let mut map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        });
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (domain, name, value) = row;
                map.entry(domain).or_default().push(format!("{}={}", name, value));
            }
        }
        map.into_iter().map(|(k, v)| (k, v.join("; "))).collect()
    }

    /// Total cookie count.
    pub fn count(&self) -> usize {
        let db = self.db.lock().unwrap();
        db.query_row("SELECT COUNT(*) FROM cookies", [], |row| row.get::<_, usize>(0))
            .unwrap_or(0)
    }

    fn delete_cookie(&self, domain: &str, path: &str, name: &str) {
        let db = self.db.lock().unwrap();
        db.execute(
            "DELETE FROM cookies WHERE domain = ?1 AND path = ?2 AND name = ?3",
            [domain, path, name],
        ).ok();
    }
}

/// Domain matching: `www.google.es` matches `.google.es` and `google.es`.
fn domain_matches(hostname: &str, cookie_domain: &str) -> bool {
    if hostname == cookie_domain {
        return true;
    }
    // hostname ends with .cookie_domain
    if hostname.ends_with(&format!(".{}", cookie_domain)) {
        return true;
    }
    // cookie_domain ends with .hostname (shouldn't happen but be permissive)
    if cookie_domain.ends_with(&format!(".{}", hostname)) {
        return true;
    }
    false
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Parse a simplified HTTP date to unix timestamp.
/// Handles: "Thu, 01 Jan 2030 00:00:00 GMT" and similar.
fn parse_http_date(s: &str) -> Result<i64, ()> {
    // Try chrono parsing
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(s) {
        return Ok(dt.timestamp());
    }
    // Try common HTTP date format
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s.trim(), "%a, %d %b %Y %H:%M:%S GMT") {
        return Ok(dt.and_utc().timestamp());
    }
    Err(())
}
