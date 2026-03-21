//! Engine configuration with sane defaults.

/// Resource limits to prevent runaway pages from consuming the host.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResourceLimits {
    /// V8 heap limit in megabytes.
    pub max_heap_mb: usize,
    /// Per-script execution timeout (ms).
    pub max_script_time_ms: u64,
    /// Total page timeout including all scripts and fetches (ms).
    pub max_total_time_ms: u64,
    /// Maximum number of concurrent in-flight fetches.
    pub max_concurrent_requests: u32,
    /// Maximum size of a single HTTP response body (bytes).
    pub max_response_bytes: usize,
    /// Absolute watchdog kill — engine is force-stopped after this (ms).
    pub watchdog_timeout_ms: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_heap_mb: 256,
            max_script_time_ms: 3_000,
            max_total_time_ms: 30_000,
            max_concurrent_requests: 10,
            max_response_bytes: 10 * 1024 * 1024, // 10 MB
            watchdog_timeout_ms: 60_000,
        }
    }
}

/// Security boundary configuration for sandboxed page JS.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SecurityConfig {
    /// Freeze Object.prototype and Array.prototype after polyfill setup.
    pub freeze_prototypes: bool,
    /// Block eval() in page JavaScript (not bootstrap).
    pub block_eval: bool,
    /// Replace Bearer tokens, cookies, and auth headers with "[REDACTED]" in trace exports.
    pub redact_auth_in_traces: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            freeze_prototypes: true,
            block_eval: false,
            redact_auth_in_traces: true,
        }
    }
}

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
    /// Resource governance limits.
    pub resource_limits: ResourceLimits,
    /// Security boundary settings.
    pub security: SecurityConfig,
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
            resource_limits: ResourceLimits::default(),
            security: SecurityConfig::default(),
        }
    }
}
