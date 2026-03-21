//! Engine configuration with sane defaults.

/// Configuration for the browser engine.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EngineConfig {
    /// Max time to wait for navigation to complete (ms).
    pub navigation_timeout_ms: u64,
    /// Max time for a single script execution (ms).
    pub script_timeout_ms: u64,
    /// Max time to wait for DOM stability after JS (ms).
    pub stability_timeout_ms: u64,
    /// Maximum number of HTTP redirects to follow.
    pub max_redirects: u32,
    /// Whether to execute page JavaScript.
    pub execute_js: bool,
    /// Whether to cache compiled JS modules.
    pub cache_modules: bool,
    /// Whether to stub heavy JS modules (analytics, etc.).
    pub stub_heavy_modules: bool,
    /// Byte threshold above which a module is considered heavy.
    pub stub_threshold_bytes: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            navigation_timeout_ms: 10_000,
            script_timeout_ms: 5_000,
            stability_timeout_ms: 3_000,
            max_redirects: 10,
            execute_js: true,
            cache_modules: true,
            stub_heavy_modules: true,
            stub_threshold_bytes: 1_000_000,
        }
    }
}
