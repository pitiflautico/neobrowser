//! SQLite-backed cookie store with SameSite context awareness.

use super::{domain_matches, extract_host, is_same_site, now_secs, parse_set_cookie, path_matches};
use crate::{CookieStore, HttpError};
use neo_types::Cookie;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Mutex;

/// Cookie store backed by SQLite for persistence across sessions.
///
/// Supports SameSite (Strict, Lax, None), path matching, and expiry.
/// DB default location: `~/.neorender/cookies.db`.
#[derive(Debug)]
pub struct SqliteCookieStore {
    conn: Mutex<Connection>,
}

impl SqliteCookieStore {
    /// Open or create a cookie store at the given path.
    pub fn open(path: &str) -> Result<Self, HttpError> {
        let p = PathBuf::from(path);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| HttpError::CookieStore(e.to_string()))?;
        }
        let conn = Connection::open(path).map_err(|e| HttpError::CookieStore(e.to_string()))?;
        create_table(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Open using the default path (`~/.neorender/cookies.db`).
    pub fn default_store() -> Result<Self, HttpError> {
        let home =
            std::env::var("HOME").map_err(|_| HttpError::CookieStore("HOME not set".into()))?;
        let path = format!("{home}/.neorender/cookies.db");
        Self::open(&path)
    }

    /// Open an in-memory database (useful for tests).
    pub fn in_memory() -> Result<Self, HttpError> {
        let conn =
            Connection::open_in_memory().map_err(|e| HttpError::CookieStore(e.to_string()))?;
        create_table(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

/// Create the cookies table if it doesn't exist.
fn create_table(conn: &Connection) -> Result<(), HttpError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS cookies (
            name TEXT NOT NULL,
            value TEXT NOT NULL,
            domain TEXT NOT NULL,
            path TEXT NOT NULL DEFAULT '/',
            expires INTEGER,
            http_only INTEGER NOT NULL DEFAULT 0,
            secure INTEGER NOT NULL DEFAULT 0,
            same_site TEXT,
            UNIQUE(name, domain, path)
        )",
    )
    .map_err(|e| HttpError::CookieStore(e.to_string()))
}

impl CookieStore for SqliteCookieStore {
    /// Build a Cookie header respecting SameSite, path, and expiry.
    fn get_for_request(
        &self,
        url: &str,
        top_level_url: Option<&str>,
        is_top_level: bool,
    ) -> String {
        let conn = self.conn.lock().expect("cookie lock poisoned");
        let host = extract_host(url);
        let path = url::Url::parse(url)
            .map(|u| u.path().to_string())
            .unwrap_or_else(|_| "/".into());
        let now = now_secs();

        let mut stmt = conn
            .prepare(
                "SELECT name, value, domain, path, same_site, expires \
                 FROM cookies WHERE (expires IS NULL OR expires > ?1)",
            )
            .expect("prepare failed");

        let rows = stmt
            .query_map([now], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .expect("query failed");

        let mut pairs = Vec::new();
        for row in rows.flatten() {
            let (name, value, domain, cpath, same_site) = row;
            if !domain_matches(&domain, &host) || !path_matches(&cpath, &path) {
                continue;
            }
            if let Some(ref ss) = same_site {
                let ss_up = ss.to_uppercase();
                if let Some(tlu) = top_level_url {
                    let same = is_same_site(url, tlu);
                    if ss_up == "STRICT" && !same {
                        continue;
                    }
                    if ss_up == "LAX" && !same && !is_top_level {
                        continue;
                    }
                }
            }
            pairs.push(format!("{name}={value}"));
        }
        pairs.join("; ")
    }

    /// Parse and store a Set-Cookie header value.
    fn store_set_cookie(&self, url: &str, set_cookie: &str) {
        let conn = self.conn.lock().expect("cookie lock poisoned");
        let host = extract_host(url);
        let cookie = parse_set_cookie(set_cookie, &host);
        let _ = conn.execute(
            "INSERT OR REPLACE INTO cookies \
             (name, value, domain, path, expires, http_only, secure, same_site) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                cookie.name,
                cookie.value,
                cookie.domain,
                cookie.path,
                cookie.expires,
                cookie.http_only as i32,
                cookie.secure as i32,
                cookie.same_site,
            ],
        );
    }

    /// Delete a specific cookie by name, domain, and path.
    fn delete(&self, name: &str, domain: &str, path: &str) {
        let conn = self.conn.lock().expect("cookie lock poisoned");
        let _ = conn.execute(
            "DELETE FROM cookies WHERE name = ?1 AND domain = ?2 AND path = ?3",
            rusqlite::params![name, domain, path],
        );
    }

    /// Remove all expired cookies from the store.
    fn evict_expired(&self) {
        let conn = self.conn.lock().expect("cookie lock poisoned");
        let _ = conn.execute(
            "DELETE FROM cookies WHERE expires IS NOT NULL AND expires <= ?1",
            [now_secs()],
        );
    }

    /// Remove all session cookies (those without an expiry).
    fn clear_session(&self) {
        let conn = self.conn.lock().expect("cookie lock poisoned");
        let _ = conn.execute("DELETE FROM cookies WHERE expires IS NULL", []);
    }

    /// List all cookies matching a domain.
    fn list_for_domain(&self, domain: &str) -> Vec<Cookie> {
        let conn = self.conn.lock().expect("cookie lock poisoned");
        query_cookies(&conn, "WHERE domain = ?1", &[domain])
    }

    /// Export all cookies from the store.
    fn export(&self) -> Vec<Cookie> {
        let conn = self.conn.lock().expect("cookie lock poisoned");
        query_cookies_all(&conn)
    }

    /// Import cookies into the store, replacing conflicts.
    fn import(&self, cookies: &[Cookie]) {
        let conn = self.conn.lock().expect("cookie lock poisoned");
        for c in cookies {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO cookies \
                 (name, value, domain, path, expires, http_only, secure, same_site) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    c.name,
                    c.value,
                    c.domain,
                    c.path,
                    c.expires,
                    c.http_only as i32,
                    c.secure as i32,
                    c.same_site,
                ],
            );
        }
    }

    /// Snapshot all cookies (same as export).
    fn snapshot(&self) -> Vec<Cookie> {
        self.export()
    }
}

/// Query all cookies from the database.
fn query_cookies_all(conn: &Connection) -> Vec<Cookie> {
    let mut stmt = conn
        .prepare(
            "SELECT name, value, domain, path, expires, \
             http_only, secure, same_site FROM cookies",
        )
        .expect("prepare failed");
    stmt.query_map([], row_to_cookie)
        .expect("query failed")
        .flatten()
        .collect()
}

/// Query cookies with a WHERE clause.
fn query_cookies(conn: &Connection, where_clause: &str, params: &[&str]) -> Vec<Cookie> {
    let sql = format!(
        "SELECT name, value, domain, path, expires, \
         http_only, secure, same_site FROM cookies {where_clause}"
    );
    let mut stmt = conn.prepare(&sql).expect("prepare failed");
    let p: Vec<&dyn rusqlite::types::ToSql> = params
        .iter()
        .map(|s| s as &dyn rusqlite::types::ToSql)
        .collect();
    stmt.query_map(p.as_slice(), row_to_cookie)
        .expect("query failed")
        .flatten()
        .collect()
}

/// Map a database row to a Cookie struct.
fn row_to_cookie(row: &rusqlite::Row) -> rusqlite::Result<Cookie> {
    Ok(Cookie {
        name: row.get(0)?,
        value: row.get(1)?,
        domain: row.get(2)?,
        path: row.get(3)?,
        expires: row.get(4)?,
        http_only: row.get::<_, i32>(5)? != 0,
        secure: row.get::<_, i32>(6)? != 0,
        same_site: row.get(7)?,
    })
}
