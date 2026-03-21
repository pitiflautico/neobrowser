//! DenoRuntime — V8-backed implementation of JsRuntime trait.
//!
//! Creates a deno_core::JsRuntime with browser polyfills (linkedom DOM),
//! ES module support via NeoModuleLoader, and V8 bytecode caching.

use crate::modules::{NeoModuleLoader, ScriptStoreHandle};
use crate::ops;
use crate::scheduler::TaskTracker;
use crate::{JsRuntime as JsRuntimeTrait, RuntimeConfig, RuntimeError};
use deno_core::{PollEventLoopOptions, RuntimeOptions};
use neo_http::HttpClient;
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

// ─── Extension declaration ───

deno_core::extension!(
    neo_runtime_ext,
    ops = [
        ops::op_fetch,
        ops::op_timer,
        ops::op_storage_get,
        ops::op_storage_set,
        ops::op_storage_remove,
        ops::op_console_log,
    ],
);

// ─── V8 Bytecode Cache ───

/// Disk-backed V8 compiled bytecode cache.
///
/// Stores compiled bytecode keyed by URL hash.
/// File format: `[8 bytes source_hash LE] [V8 bytecode...]`
pub struct V8CodeCache {
    cache_dir: PathBuf,
}

impl V8CodeCache {
    /// Create a new code cache at the given directory.
    pub fn new(cache_dir: &PathBuf) -> Result<Self, std::io::Error> {
        std::fs::create_dir_all(cache_dir)?;
        Ok(Self {
            cache_dir: cache_dir.clone(),
        })
    }

    /// Hash source code for cache invalidation.
    pub fn hash_source(code: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        code.hash(&mut hasher);
        hasher.finish()
    }

    /// Try to read cached bytecode. Returns None if missing or stale.
    pub fn read(&self, url: &str, source_hash: u64) -> Option<Vec<u8>> {
        let path = self.cache_path(url);
        let data = std::fs::read(&path).ok()?;
        if data.len() < 8 {
            return None;
        }
        let stored = u64::from_le_bytes(data[..8].try_into().ok()?);
        if stored != source_hash {
            return None;
        }
        Some(data[8..].to_vec())
    }

    /// Write bytecode to disk with source hash prefix.
    pub fn write(&self, url: &str, source_hash: u64, bytecode: &[u8]) {
        let path = self.cache_path(url);
        let mut data = Vec::with_capacity(8 + bytecode.len());
        data.extend_from_slice(&source_hash.to_le_bytes());
        data.extend_from_slice(bytecode);
        let _ = std::fs::write(&path, &data);
    }

    /// Deterministic filename from URL.
    fn cache_path(&self, url: &str) -> PathBuf {
        let mut hasher = DefaultHasher::new();
        url.hash(&mut hasher);
        self.cache_dir
            .join(format!("{:016x}.v8cache", hasher.finish()))
    }
}

// ─── DenoRuntime ───

/// V8-backed JavaScript runtime using deno_core.
pub struct DenoRuntime {
    /// The underlying deno_core runtime.
    runtime: deno_core::JsRuntime,
    /// Shared script store for module loading.
    store: ScriptStoreHandle,
    /// Task tracker for pending async work.
    tracker: TaskTracker,
    /// Tokio runtime for blocking on async ops.
    tokio_rt: tokio::runtime::Runtime,
}

// SAFETY: DenoRuntime is only used from a single thread at a time.
// deno_core::JsRuntime is !Send due to V8 isolate, but we ensure
// single-threaded access through our API.
unsafe impl Send for DenoRuntime {}

impl DenoRuntime {
    /// Create a new V8 runtime with the given configuration.
    pub fn new(config: &RuntimeConfig) -> Result<Self, RuntimeError> {
        Self::new_inner(config, None)
    }

    /// Create a new V8 runtime with an HttpClient for op_fetch.
    ///
    /// The HttpClient is stored in OpState so that JavaScript `fetch()`
    /// calls route through the real HTTP layer.
    pub fn new_with_http(
        config: &RuntimeConfig,
        http_client: Arc<dyn HttpClient>,
    ) -> Result<Self, RuntimeError> {
        Self::new_inner(config, Some(http_client))
    }

    fn new_inner(
        config: &RuntimeConfig,
        http_client: Option<Arc<dyn HttpClient>>,
    ) -> Result<Self, RuntimeError> {
        let store = Rc::new(RefCell::new(crate::modules::ScriptStore::default()));

        let code_cache = config
            .cache_dir
            .as_ref()
            .and_then(|dir| V8CodeCache::new(dir).ok())
            .map(Rc::new);

        let loader = NeoModuleLoader {
            store: store.clone(),
            code_cache,
        };

        let mut runtime = deno_core::JsRuntime::new(RuntimeOptions {
            extensions: vec![neo_runtime_ext::init_ops()],
            module_loader: Some(Rc::new(loader)),
            ..Default::default()
        });

        // Put HttpClient and other state in OpState for ops.
        {
            let op_state = runtime.op_state();
            let mut state = op_state.borrow_mut();
            if let Some(client) = http_client {
                state.put(ops::SharedHttpClient(client));
            }
            state.put(ops::ConsoleBuffer::default());
            state.put(ops::StorageState::default());
        }

        // Node.js polyfills required by linkedom (Buffer, process, atob/btoa).
        let node_polyfills: &str = include_str!("../../../js/node_polyfills.js");
        runtime
            .execute_script("<neorender:node_polyfills>", node_polyfills.to_string())
            .map_err(|e| {
                RuntimeError::Init(format!(
                    "node polyfills: {}",
                    first_line(&e.to_string())
                ))
            })?;

        // Load linkedom DOM implementation (included at compile time).
        let linkedom_js: &str = include_str!("../../../js/linkedom.js");
        runtime
            .execute_script("<neorender:linkedom>", linkedom_js.to_string())
            .map_err(|e| {
                RuntimeError::Init(format!("linkedom load: {}", first_line(&e.to_string())))
            })?;

        let tokio_rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| RuntimeError::Init(e.to_string()))?;

        Ok(Self {
            runtime,
            store,
            tracker: TaskTracker::new(),
            tokio_rt,
        })
    }

    /// Access the shared script store to add pre-fetched scripts.
    pub fn script_store(&self) -> &ScriptStoreHandle {
        &self.store
    }

    /// Access the task tracker.
    pub fn tracker(&self) -> &TaskTracker {
        &self.tracker
    }
}

impl JsRuntimeTrait for DenoRuntime {
    fn eval(&mut self, code: &str) -> Result<String, RuntimeError> {
        let wrapped = format!(
            "try {{ String({}) }} catch(__e) {{ 'Error: ' + __e.message }}",
            code
        );
        let result = self
            .runtime
            .execute_script("<eval>", wrapped)
            .map_err(|e| RuntimeError::Eval(first_line(&e.to_string())))?;

        let scope = &mut self.runtime.handle_scope();
        let local = deno_core::v8::Local::new(scope, result);
        if let Some(s) = local.to_string(scope) {
            Ok(s.to_rust_string_lossy(scope))
        } else {
            Ok("undefined".to_string())
        }
    }

    fn execute(&mut self, code: &str) -> Result<(), RuntimeError> {
        let wrapped = format!(
            "try {{ {} }} catch(__e) {{ /* non-fatal */ }}",
            code
        );
        self.runtime
            .execute_script("<script>", wrapped)
            .map_err(|e| RuntimeError::Eval(first_line(&e.to_string())))?;
        Ok(())
    }

    fn load_module(&mut self, url: &str) -> Result<(), RuntimeError> {
        let specifier = deno_core::ModuleSpecifier::parse(url)
            .map_err(|e| RuntimeError::Module(e.to_string()))?;

        self.tokio_rt.block_on(async {
            let mod_id = self
                .runtime
                .load_main_es_module(&specifier)
                .await
                .map_err(|e| RuntimeError::Module(first_line(&e.to_string())))?;

            let eval = self.runtime.mod_evaluate(mod_id);

            self.runtime
                .run_event_loop(PollEventLoopOptions::default())
                .await
                .map_err(|e| {
                    RuntimeError::Module(format!("event loop: {}", first_line(&e.to_string())))
                })?;

            eval.await
                .map_err(|e| RuntimeError::Module(first_line(&e.to_string())))?;

            Ok(())
        })
    }

    fn run_until_settled(&mut self, timeout_ms: u64) -> Result<(), RuntimeError> {
        self.tokio_rt.block_on(async {
            match tokio::time::timeout(
                std::time::Duration::from_millis(timeout_ms),
                self.runtime.run_event_loop(PollEventLoopOptions::default()),
            )
            .await
            {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => {
                    // Non-fatal event loop errors (React internals, etc.)
                    eprintln!(
                        "[neo-runtime] event loop error (non-fatal): {}",
                        first_line(&e.to_string())
                    );
                    Ok(())
                }
                Err(_) => Err(RuntimeError::Timeout {
                    timeout_ms,
                    pending: self.tracker.pending(),
                }),
            }
        })
    }

    fn pending_tasks(&self) -> usize {
        self.tracker.pending()
    }

    fn set_document_html(&mut self, html: &str, url: &str) -> Result<(), RuntimeError> {
        let escaped = html
            .replace('\\', "\\\\")
            .replace('`', "\\`")
            .replace("${", "\\${");
        let escaped_url = url.replace('\'', "\\'");
        let js = format!(
            "globalThis.__neorender_html = `{}`;\
             globalThis.__neorender_url = '{}';",
            escaped, escaped_url
        );
        self.runtime
            .execute_script("<set_document_html>", js)
            .map_err(|e| RuntimeError::Dom(first_line(&e.to_string())))?;

        // Load bootstrap.js — parses HTML via linkedom, sets up browser globals
        // (fetch, timers, console, DOM constructors, etc.).
        let bootstrap_js: &str = include_str!("../../../js/bootstrap.js");
        self.runtime
            .execute_script("<neorender:bootstrap>", bootstrap_js.to_string())
            .map_err(|e| {
                RuntimeError::Dom(format!("bootstrap: {}", first_line(&e.to_string())))
            })?;

        // Set location to match the page URL.
        let loc_js = format!(
            "try {{\
                const __u = new URL('{}');\
                location.href = __u.href;\
                location.protocol = __u.protocol;\
                location.host = __u.host;\
                location.hostname = __u.hostname;\
                location.port = __u.port;\
                location.pathname = __u.pathname;\
                location.search = __u.search;\
                location.hash = __u.hash;\
                location.origin = __u.origin;\
             }} catch(e) {{}}",
            escaped_url
        );
        self.runtime
            .execute_script("<set_location>", loc_js)
            .map_err(|e| RuntimeError::Dom(first_line(&e.to_string())))?;

        Ok(())
    }

    fn export_html(&mut self) -> Result<String, RuntimeError> {
        self.eval("globalThis.__neorender_export ? __neorender_export() : ''")
    }
}

/// Extract the first line of an error message.
fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).to_string()
}
