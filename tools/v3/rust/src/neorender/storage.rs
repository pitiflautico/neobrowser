//! Persistent localStorage backed by SQLite.
//! Each domain gets its own key-value namespace, persisted across sessions.

use rusqlite::Connection;

pub struct BrowserStorage {
    conn: Connection,
}

impl BrowserStorage {
    pub fn new() -> Result<Self, String> {
        let storage_dir = dirs::home_dir()
            .map(|h| h.join(".neobrowser").join("storage"))
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp/neobrowser-storage"));
        std::fs::create_dir_all(&storage_dir).ok();
        let db_path = storage_dir.join("localStorage.db");
        let conn = Connection::open(&db_path).map_err(|e| format!("Storage DB: {e}"))?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS storage (domain TEXT, key TEXT, value TEXT, PRIMARY KEY (domain, key))",
            [],
        ).map_err(|e| format!("Storage table: {e}"))?;
        Ok(Self { conn })
    }

    pub fn get(&self, domain: &str, key: &str) -> Option<String> {
        self.conn.query_row(
            "SELECT value FROM storage WHERE domain = ? AND key = ?",
            [domain, key],
            |row| row.get(0),
        ).ok()
    }

    pub fn set(&self, domain: &str, key: &str, value: &str) -> Result<(), String> {
        self.conn.execute(
            "INSERT OR REPLACE INTO storage (domain, key, value) VALUES (?, ?, ?)",
            [domain, key, value],
        ).map_err(|e| format!("Storage set: {e}"))?;
        Ok(())
    }

    pub fn remove(&self, domain: &str, key: &str) -> Result<(), String> {
        self.conn.execute("DELETE FROM storage WHERE domain = ? AND key = ?", [domain, key])
            .map_err(|e| format!("Storage remove: {e}"))?;
        Ok(())
    }

    pub fn clear(&self, domain: &str) -> Result<(), String> {
        self.conn.execute("DELETE FROM storage WHERE domain = ?", [domain])
            .map_err(|e| format!("Storage clear: {e}"))?;
        Ok(())
    }

    pub fn keys(&self, domain: &str) -> Vec<String> {
        let Ok(mut stmt) = self.conn.prepare("SELECT key FROM storage WHERE domain = ?") else {
            return Vec::new();
        };
        let Ok(rows) = stmt.query_map([domain], |row| row.get(0)) else {
            return Vec::new();
        };
        rows.filter_map(|r| r.ok()).collect()
    }

    pub fn get_all(&self, domain: &str) -> Vec<(String, String)> {
        let Ok(mut stmt) = self.conn.prepare("SELECT key, value FROM storage WHERE domain = ?") else {
            return Vec::new();
        };
        let Ok(rows) = stmt.query_map([domain], |row| Ok((row.get(0)?, row.get(1)?))) else {
            return Vec::new();
        };
        rows.filter_map(|r| r.ok()).collect()
    }

    /// Load all entries for a domain into a HashMap (for injecting into V8)
    pub fn to_hashmap(&self, domain: &str) -> std::collections::HashMap<String, String> {
        self.get_all(domain).into_iter().collect()
    }
}
