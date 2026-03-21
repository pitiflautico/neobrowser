//! SQLite-backed web storage (localStorage / sessionStorage).
//!
//! Uses the same DB as cookies (`~/.neorender/cookies.db`), different table.
//! localStorage persists across sessions; sessionStorage is cleared on session destroy.

use crate::{HttpError, WebStorage};
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageType {
    Local,
    Session,
}

impl StorageType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Session => "session",
        }
    }
}

#[derive(Debug)]
pub struct SqliteWebStorage {
    conn: Mutex<Connection>,
    storage_type: StorageType,
}

fn create_table(conn: &Connection) -> Result<(), HttpError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS storage (
            origin       TEXT NOT NULL,
            key          TEXT NOT NULL,
            value        TEXT NOT NULL,
            storage_type TEXT NOT NULL DEFAULT 'local',
            PRIMARY KEY (origin, key, storage_type)
        );",
    )
    .map_err(|e| HttpError::CookieStore(e.to_string()))
}

impl SqliteWebStorage {
    pub fn open(path: &str, storage_type: StorageType) -> Result<Self, HttpError> {
        let p = PathBuf::from(path);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| HttpError::CookieStore(e.to_string()))?;
        }
        let conn = Connection::open(path).map_err(|e| HttpError::CookieStore(e.to_string()))?;
        create_table(&conn)?;
        Ok(Self { conn: Mutex::new(conn), storage_type })
    }

    pub fn default_local() -> Result<Self, HttpError> {
        let home = std::env::var("HOME").map_err(|_| HttpError::CookieStore("HOME not set".into()))?;
        Self::open(&format!("{home}/.neorender/cookies.db"), StorageType::Local)
    }

    pub fn default_session() -> Result<Self, HttpError> {
        let home = std::env::var("HOME").map_err(|_| HttpError::CookieStore("HOME not set".into()))?;
        Self::open(&format!("{home}/.neorender/cookies.db"), StorageType::Session)
    }

    pub fn in_memory(storage_type: StorageType) -> Result<Self, HttpError> {
        let conn = Connection::open_in_memory().map_err(|e| HttpError::CookieStore(e.to_string()))?;
        create_table(&conn)?;
        Ok(Self { conn: Mutex::new(conn), storage_type })
    }

    pub fn clear_session_storage(&self) {
        let conn = self.conn.lock().expect("lock poisoned");
        let _ = conn.execute("DELETE FROM storage WHERE storage_type = 'session'", []);
    }
}

impl WebStorage for SqliteWebStorage {
    fn get(&self, origin: &str, key: &str) -> Option<String> {
        let conn = self.conn.lock().expect("lock poisoned");
        conn.query_row(
            "SELECT value FROM storage WHERE origin = ?1 AND key = ?2 AND storage_type = ?3",
            rusqlite::params![origin, key, self.storage_type.as_str()],
            |row| row.get(0),
        ).ok()
    }

    fn set(&self, origin: &str, key: &str, value: &str) {
        let conn = self.conn.lock().expect("lock poisoned");
        let _ = conn.execute(
            "INSERT OR REPLACE INTO storage (origin, key, value, storage_type) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![origin, key, value, self.storage_type.as_str()],
        );
    }

    fn remove(&self, origin: &str, key: &str) {
        let conn = self.conn.lock().expect("lock poisoned");
        let _ = conn.execute(
            "DELETE FROM storage WHERE origin = ?1 AND key = ?2 AND storage_type = ?3",
            rusqlite::params![origin, key, self.storage_type.as_str()],
        );
    }

    fn clear(&self, origin: &str) {
        let conn = self.conn.lock().expect("lock poisoned");
        let _ = conn.execute(
            "DELETE FROM storage WHERE origin = ?1 AND storage_type = ?2",
            rusqlite::params![origin, self.storage_type.as_str()],
        );
    }

    fn keys(&self, origin: &str) -> Vec<String> {
        let conn = self.conn.lock().expect("lock poisoned");
        let mut stmt = match conn.prepare(
            "SELECT key FROM storage WHERE origin = ?1 AND storage_type = ?2 ORDER BY key",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map(
            rusqlite::params![origin, self.storage_type.as_str()],
            |row| row.get(0),
        )
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    fn len(&self, origin: &str) -> usize {
        let conn = self.conn.lock().expect("lock poisoned");
        conn.query_row(
            "SELECT COUNT(*) FROM storage WHERE origin = ?1 AND storage_type = ?2",
            rusqlite::params![origin, self.storage_type.as_str()],
            |row| row.get::<_, usize>(0),
        ).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_storage_persists() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let path = db.to_str().unwrap();
        {
            let store = SqliteWebStorage::open(path, StorageType::Local).unwrap();
            store.set("https://example.com", "token", "abc123");
        }
        {
            let store = SqliteWebStorage::open(path, StorageType::Local).unwrap();
            assert_eq!(store.get("https://example.com", "token"), Some("abc123".to_string()));
        }
    }

    #[test]
    fn test_session_storage_cleared() {
        let store = SqliteWebStorage::in_memory(StorageType::Session).unwrap();
        store.set("https://example.com", "sid", "xyz");
        assert_eq!(store.get("https://example.com", "sid"), Some("xyz".to_string()));
        store.clear_session_storage();
        assert_eq!(store.get("https://example.com", "sid"), None);
    }

    #[test]
    fn test_origin_isolation() {
        let store = SqliteWebStorage::in_memory(StorageType::Local).unwrap();
        store.set("https://a.com", "key", "val_a");
        assert_eq!(store.get("https://b.com", "key"), None);
        assert_eq!(store.get("https://a.com", "key"), Some("val_a".to_string()));
    }

    #[test]
    fn test_keys_and_len() {
        let store = SqliteWebStorage::in_memory(StorageType::Local).unwrap();
        let origin = "https://example.com";
        store.set(origin, "a", "1");
        store.set(origin, "b", "2");
        store.set(origin, "c", "3");
        assert_eq!(store.len(origin), 3);
        let mut k = store.keys(origin);
        k.sort();
        assert_eq!(k, vec!["a", "b", "c"]);
    }
}
