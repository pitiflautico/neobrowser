//! JsRuntime trait implementation for DenoRuntime.

use deno_core::PollEventLoopOptions;
use std::time::{Duration, Instant};

use crate::event_loop::{EventLoopRunner, PumpResult, SettleConfig, SettleReason};
use crate::neo_trace;
use crate::trace_events::{ModulePhase, TraceBuffer};
use crate::v8::DenoRuntime;
use crate::{EvalSettleResult, JsRuntime as JsRuntimeTrait, RuntimeError, RuntimeHandle};

/// Extract the first line of an error message.
pub(crate) fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).to_string()
}

/// Get a clone of the TraceBuffer from OpState.
fn get_trace_buffer(runtime: &deno_core::JsRuntime) -> TraceBuffer {
    let op_state = runtime.op_state();
    let state = op_state.borrow();
    state
        .try_borrow::<TraceBuffer>()
        .cloned()
        .unwrap_or_default()
}

// ─── Shared helpers ───

/// Create a V8 HandleScope + ContextScope from a JsRuntime.
///
/// Replacement for the removed `JsRuntime::handle_scope()` method.
/// Uses the `v8::scope!` macro pattern with `main_context()`.
///
/// Usage: `neo_handle_scope!(scope, runtime);`
/// After the macro, `scope` is a `&mut ContextScope<HandleScope>`.
macro_rules! neo_handle_scope {
    ($scope:ident, $runtime:expr) => {
        let context = $runtime.main_context();
        deno_core::v8::scope!($scope, $runtime.v8_isolate());
        let context = deno_core::v8::Local::new($scope, context);
        let $scope = &mut deno_core::v8::ContextScope::new($scope, context);
    };
}

pub(crate) use neo_handle_scope;

impl DenoRuntime {
    /// Convert a V8 global value to a Rust string.
    fn v8_value_to_string(&mut self, global: deno_core::v8::Global<deno_core::v8::Value>) -> String {
        neo_handle_scope!(scope, self.runtime);
        let local = deno_core::v8::Local::new(scope, global);
        local
            .to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope))
            .unwrap_or_else(|| "undefined".to_string())
    }

    /// Convert a V8 global ref to a Rust string (borrows the global, does not consume).
    fn v8_value_ref_to_string(&mut self, global: &deno_core::v8::Global<deno_core::v8::Value>) -> String {
        neo_handle_scope!(scope, self.runtime);
        let local = deno_core::v8::Local::new(scope, global);
        local
            .to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope))
            .unwrap_or_else(|| "undefined".to_string())
    }

    /// Drain the V8 microtask queue (Chromium kExplicit pattern).
    fn drain_microtasks(&mut self) {
        // In V8 146+, perform_microtask_checkpoint is on Isolate directly.
        self.runtime.v8_isolate().perform_microtask_checkpoint();
    }

    /// Resolve a V8 Promise global using a disposable tokio runtime.
    ///
    /// Uses a SEPARATE tokio runtime to avoid nested block_on on self.tokio_rt,
    /// which causes web_timeout.rs:189 data race panics.
    fn resolve_promise_value(
        &mut self,
        global: deno_core::v8::Global<deno_core::v8::Value>,
        timeout_ms: u64,
    ) -> Result<String, PromiseResolveError> {
        let resolve_rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| PromiseResolveError::Runtime(e.to_string()))?;

        #[allow(deprecated)]
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            resolve_rt.block_on(async {
                tokio::time::timeout(
                    Duration::from_millis(timeout_ms),
                    self.runtime.resolve_value(global),
                )
                .await
            })
        }));

        match result {
            Ok(Ok(Ok(resolved))) => Ok(self.v8_value_to_string(resolved)),
            Ok(Ok(Err(e))) => Err(PromiseResolveError::Eval(first_line(&e.to_string()))),
            Ok(Err(_)) => Err(PromiseResolveError::Timeout(timeout_ms)),
            Err(_) => {
                eprintln!("[neo-runtime] resolve_promise_value: PANIC caught during resolve_value");
                Err(PromiseResolveError::Panic)
            }
        }
    }

    /// Execute JS wrapped in try/catch, return the raw V8 global value.
    fn eval_raw(&mut self, code: &str) -> Result<deno_core::v8::Global<deno_core::v8::Value>, RuntimeError> {
        let wrapped = format!(
            "try {{ (\n{}\n) }} catch(__e) {{ 'Error: ' + __e.message }}",
            code
        );
        self.runtime
            .execute_script("<eval-settle>", wrapped)
            .map_err(|e| RuntimeError::Eval(first_line(&e.to_string())))
    }

    /// Check whether a V8 global value is a Promise.
    fn is_promise(&mut self, global: &deno_core::v8::Global<deno_core::v8::Value>) -> bool {
        neo_handle_scope!(scope, self.runtime);
        let local = deno_core::v8::Local::new(scope, global);
        local.is_promise()
    }

    /// Query the pending timers count from JS.
    fn query_pending_timers(&mut self) -> usize {
        self.eval("typeof __neo_pendingTimers==='function'?__neo_pendingTimers():0")
            .unwrap_or_default()
            .trim()
            .parse::<usize>()
            .unwrap_or(0)
    }

    /// Settle after a Promise resolution: run event loop briefly, query pending timers.
    fn settle_after_eval(
        &mut self,
        start: Instant,
        timeout_ms: u64,
        value: String,
    ) -> EvalSettleResult {
        let remaining = timeout_ms.saturating_sub(start.elapsed().as_millis() as u64);
        if remaining > 100 {
            let _ = self.run_until_settled(remaining.min(1000));
        }
        let pending = self.query_pending_timers();
        EvalSettleResult {
            value,
            was_promise: true,
            settled_ms: start.elapsed().as_millis() as u64,
            pending_timers: pending,
        }
    }
}

/// Internal error type for promise resolution — avoids polluting RuntimeError
/// with intermediate states that only eval_promise/eval_and_settle care about.
enum PromiseResolveError {
    /// Failed to create disposable tokio runtime.
    Runtime(String),
    /// JS evaluation error during resolution.
    Eval(String),
    /// Resolution timed out.
    Timeout(u64),
    /// V8 panic during resolution.
    Panic,
}

// ─── Module loading helpers (decomposed from load_module) ───

impl DenoRuntime {
    /// Notify JS that a module load is starting.
    fn notify_module_requested(&mut self, url: &str) {
        let escaped = url.replace('\'', "\\'");
        let _ = self.runtime.execute_script(
            "<module-track-req>",
            format!("typeof __neo_moduleRequested==='function'&&__neo_moduleRequested('{escaped}')"),
        );
    }

    /// Notify JS that a module failed to load/evaluate.
    fn notify_module_failed(&mut self, url: &str, reason: &str) {
        let escaped = url.replace('\'', "\\'");
        let _ = self.runtime.execute_script(
            "<module-track-fail>",
            format!("typeof __neo_moduleFailed==='function'&&__neo_moduleFailed('{escaped}','{reason}')"),
        );
    }

    /// Notify JS that a module loaded successfully.
    fn notify_module_loaded(&mut self, url: &str) {
        let escaped = url.replace('\'', "\\'");
        let _ = self.runtime.execute_script(
            "<module-track-ok>",
            format!("typeof __neo_moduleLoaded==='function'&&__neo_moduleLoaded('{escaped}')"),
        );
    }

    /// Load (fetch + instantiate) an ES module. Returns the mod_id.
    ///
    /// Caller must have entered the tokio context (`_guard = tokio_rt.enter()`)
    /// before calling this — deno_core's WebTimers need the tokio reactor
    /// for `tokio::time::sleep_until` during module instantiation.
    fn load_es_module(
        &mut self,
        specifier: &deno_core::ModuleSpecifier,
        url: &str,
    ) -> Result<usize, RuntimeError> {
        let result = self.tokio_rt.block_on(async {
            self.runtime.load_side_es_module(specifier).await
        });

        match result {
            Ok(id) => Ok(id),
            Err(e) => {
                let msg = first_line(&e.to_string());
                neo_trace!("[MODULE-LIFECYCLE] load_module LOAD-FAILED: {url} — {msg}");
                self.notify_module_failed(url, "load-failed");
                Err(RuntimeError::Module(msg))
            }
        }
    }

    /// Evaluate a module: mod_evaluate + event loop + await eval promise.
    ///
    /// This is the dangerous part — `mod_evaluate` panics if the module was
    /// already evaluated as a transitive dependency. The caller must check
    /// `module_evaluator.should_evaluate()` before calling this.
    fn evaluate_module(
        &mut self,
        mod_id: usize,
        url: &str,
        load_start: Instant,
    ) -> Result<(), RuntimeError> {
        let tb = get_trace_buffer(&self.runtime);
        tb.module_event(url, ModulePhase::Evaluate, None);

        // catch_unwind around mod_evaluate — it panics on "already evaluated"
        let eval_handle = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.runtime.mod_evaluate(mod_id)
        })) {
            Ok(handle) => handle,
            Err(_) => {
                self.module_evaluator.mark_corrupted();
                neo_trace!("[MODULE-LIFECYCLE] mod_evaluate PANICKED for {url} (mod_id={mod_id}) — isolate corrupted");
                tb.module_event(url, ModulePhase::Error, Some("mod_evaluate panicked — isolate corrupted"));
                return Ok(()); // non-fatal
            }
        };

        // Run event loop with 15s timeout — large SPAs need time for transitive deps.
        // If the module threw at top level, the event loop surfaces the error.
        let event_loop_err = self.run_module_event_loop(url, load_start).err();

        // Await evaluation promise (1s timeout).
        let eval_err = self.await_eval_promise(eval_handle, url, load_start).err();

        // Prefer eval error over event loop error (more specific), but return whichever exists.
        if let Some(e) = eval_err.or(event_loop_err) {
            tb.module_event(url, ModulePhase::Error, Some(&e.to_string()));
            return Err(e);
        }

        // Mark as evaluated. After eval + event loop, deno_core may have loaded
        // and evaluated transitive dependencies with HIGHER mod_ids than ours.
        // We discover this by trying to load_side_es_module for URLs we'll
        // encounter later — but we don't know those URLs here.
        //
        // Instead, we mark ALL mod_ids up to mod_id (inclusive) as deps.
        // For deps with HIGHER ids (like vendor.js=4 as dep of nuvo-importer=3),
        // the catch_unwind in mod_evaluate + module_eval_corrupted flag will
        // prevent the process from crashing. The corrupted flag skips all
        // subsequent evals, which is acceptable — the modules that DID evaluate
        // already ran their side effects.
        self.module_evaluator.mark_evaluated(mod_id);
        for dep_id in 0..=mod_id {
            self.module_evaluator.mark_transitive_dep(dep_id);
        }

        tb.module_event(url, ModulePhase::Success, Some(&format!("{}ms", load_start.elapsed().as_millis())));
        Ok(())
    }

    /// Run the event loop after mod_evaluate, with a 15s timeout and panic recovery.
    ///
    /// Returns `Err(RuntimeError::Module(...))` if the event loop surfaced a
    /// module-level error (e.g. top-level `throw`).
    fn run_module_event_loop(&mut self, url: &str, load_start: Instant) -> Result<(), RuntimeError> {
        // 200ms per-module event loop. Just enough for the module code to execute.
        // Timers (React scheduler, analytics) will be pumped during global settle.
        let pump = EventLoopRunner::pump_once(&mut self.runtime, &self.tokio_rt, 200);
        match pump {
            PumpResult::Idle => {
                neo_trace!(
                    "[MODULE-LIFECYCLE] load_module EVENT-LOOP-IDLE: {url} ({}ms)",
                    load_start.elapsed().as_millis()
                );
                Ok(())
            }
            PumpResult::Error(msg) => {
                neo_trace!("[MODULE-LIFECYCLE] load_module EVENT-LOOP-ERROR: {url} — {msg}");
                self.notify_module_failed(url, "event-loop-error");
                let short = first_line(&msg);
                // Log to JS console
                let _ = self.runtime.execute_script(
                    "<module-error>",
                    format!("try {{ console.error('[MODULE-ERROR] {}'); }} catch {{}}", short.replace('\'', "\\'")),
                );
                Err(RuntimeError::Module(short))
            }
            PumpResult::Timeout => {
                neo_trace!("[MODULE-LIFECYCLE] load_module EVENT-LOOP-TIMEOUT: {url} (15000ms)");
                Ok(()) // timeout is non-fatal
            }
            PumpResult::Panic => {
                neo_trace!("[MODULE-LIFECYCLE] load_module EVENT-LOOP-PANIC: {url} — isolate may be corrupted");
                self.notify_module_failed(url, "event-loop-panic");
                Ok(()) // panic recovery is non-fatal (isolate may be corrupted but we keep going)
            }
        }
    }

    /// Await the evaluation promise returned by mod_evaluate.
    ///
    /// Uses manual polling instead of block_on to avoid nested block_on
    /// conflicts with deno_core's WebTimer system.
    ///
    /// Returns `Err(RuntimeError::Module(...))` if the module threw at top level.
    fn await_eval_promise<E: std::fmt::Display>(
        &mut self,
        eval_handle: impl std::future::Future<Output = Result<(), E>>,
        url: &str,
        load_start: Instant,
    ) -> Result<(), RuntimeError> {
        let deadline = Instant::now() + Duration::from_millis(1000);
        let _guard = self.tokio_rt.enter();

        let mut pinned = std::pin::pin!(eval_handle);
        let waker = futures::task::noop_waker();
        let mut cx = std::task::Context::from_waker(&waker);

        let result = loop {
            match pinned.as_mut().poll(&mut cx) {
                std::task::Poll::Ready(r) => break Some(r),
                std::task::Poll::Pending => {
                    if Instant::now() >= deadline {
                        break None; // timeout
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        };

        match result {
            Some(Ok(())) => {
                neo_trace!(
                    "[MODULE-LIFECYCLE] load_module EVALUATED: {url} ({}ms total)",
                    load_start.elapsed().as_millis()
                );
                Ok(())
            }
            Some(Err(e)) => {
                let full_msg = e.to_string();
                let msg = first_line(&full_msg);
                if full_msg.contains("does not provide an export named") {
                    neo_trace!("[MODULE-LIFECYCLE] load_module EVAL-FAILED: {url} — MISSING EXPORT");
                } else {
                    neo_trace!("[MODULE-LIFECYCLE] load_module EVAL-FAILED: {url} — {msg}");
                }
                self.notify_module_failed(url, "eval-failed");
                // Log to JS console so onerror handlers can see it
                let _ = self.runtime.execute_script(
                    "<module-error>",
                    format!("try {{ console.error('[MODULE-ERROR] {}'); }} catch {{}}", msg.replace('\'', "\\'")),
                );
                Err(RuntimeError::Module(msg))
            }
            None => {
                neo_trace!("[MODULE-LIFECYCLE] load_module EVAL-TIMEOUT: {url} (1000ms)");
                Ok(()) // timeout is non-fatal
            }
        }
    }
}

// ─── set_document_html helpers (decomposed into bootstrap vs reinit) ───

impl DenoRuntime {
    /// Escape HTML for embedding in a JS template literal.
    fn escape_html_for_js(html: &str) -> String {
        html.replace('\\', "\\\\")
            .replace('`', "\\`")
            .replace("${", "\\${")
    }

    /// First-time initialization: full bootstrap.js + browser_shim.js + polyfills.
    ///
    /// Sets up the DOM environment from scratch. Cannot be re-run because
    /// bootstrap.js uses `const` declarations that are scoped to the V8 context.
    fn bootstrap_runtime(&mut self, escaped: &str, escaped_url: &str) -> Result<(), RuntimeError> {
        let trace_flag = if crate::trace::is_trace_enabled() { "true" } else { "false" };
        let js = format!(
            "globalThis.__neorender_html = `{}`;\
             globalThis.__neorender_url = '{}';\
             globalThis.__neorender_trace = {};",
            escaped, escaped_url, trace_flag
        );
        self.runtime
            .execute_script("<set_document_html>", js)
            .map_err(|e| RuntimeError::Dom(first_line(&e.to_string())))?;

        let bootstrap_js: &str = include_str!("../../../js/bootstrap.js");
        self.runtime
            .execute_script("<neorender:bootstrap>", bootstrap_js.to_string())
            .map_err(|e| {
                RuntimeError::Dom(format!("bootstrap: {}", first_line(&e.to_string())))
            })?;

        let browser_shim_js: &str = include_str!("../../../js/browser_shim.js");
        self.runtime
            .execute_script("<neorender:browser_shim>", browser_shim_js.to_string())
            .map_err(|e| {
                RuntimeError::Dom(format!("browser_shim: {}", first_line(&e.to_string())))
            })?;

        // CRITICAL: Promise.prototype.finally polyfill.
        // deno_core 0.311 V8 Promises lack .finally. Set as non-configurable
        // to prevent page scripts from accidentally deleting it.
        self.runtime.execute_script("<promise-finally>", r#"
            Object.defineProperty(Promise.prototype, 'finally', {
                value: function(onFinally) {
                    return this.then(
                        function(v) { return Promise.resolve(onFinally()).then(function() { return v; }); },
                        function(r) { return Promise.resolve(onFinally()).then(function() { throw r; }); }
                    );
                },
                writable: false, configurable: false, enumerable: false
            });
        "#.to_string()).ok();

        // Mark runtime as initialized.
        self.runtime.execute_script("<neo-init>",
            "globalThis.__neo_initialized = true;".to_string()).ok();

        // Hydration trace — monitors DOM mutations after initial load.
        let hydration_tracer = r#"
            window.__neorender_trace = window.__neorender_trace || {};
            window.__neorender_trace.modulesLoaded = 0;
            window.__neorender_trace.lastMutationTime = Date.now();
            window.__neorender_trace.mutationCount = 0;
            window.__neorender_trace.hydrationStartNodes = document.querySelectorAll('*').length;
            try {
                new MutationObserver(function(mutations) {
                    window.__neorender_trace.lastMutationTime = Date.now();
                    window.__neorender_trace.mutationCount += mutations.length;
                }).observe(document.body || document.documentElement, {
                    childList: true, subtree: true, attributes: true
                });
            } catch(e) {}
        "#;
        self.runtime
            .execute_script("<neorender:hydration_trace>", hydration_tracer.to_string())
            .map_err(|e| {
                RuntimeError::Dom(format!(
                    "hydration_trace: {}",
                    first_line(&e.to_string())
                ))
            })?;

        Ok(())
    }

    /// Subsequent calls: re-parse HTML via happy-dom and replace document content.
    fn reinit_document(&mut self, escaped: &str, escaped_url: &str) -> Result<(), RuntimeError> {
        let reinit_js = format!(
            "(function() {{\
                 globalThis.__neorender_html = `{}`;\
                 globalThis.__neorender_url = '{}';\
                 var __tmp = __linkedom_parseHTML(globalThis.__neorender_html);\
                 document.documentElement.innerHTML = __tmp.document.documentElement.innerHTML;\
                 try {{ Object.defineProperty(document, 'currentScript', {{ value: null, writable: true, configurable: true }}); }} catch {{}}\
                 try {{ document.defaultView = globalThis; }} catch {{}}\
             }})()",
            escaped, escaped_url
        );
        self.runtime
            .execute_script("<reinit_document>", reinit_js)
            .map_err(|e| RuntimeError::Dom(first_line(&e.to_string())))?;
        Ok(())
    }

    /// Set location.href and related properties via internal helper.
    fn set_location(&mut self, escaped_url: &str) -> Result<(), RuntimeError> {
        let loc_js = format!(
            "try {{\
                if (typeof globalThis.__neo_setLocationHref === 'function') {{\
                    globalThis.__neo_setLocationHref('{}');\
                }} else {{\
                    const __u = new URL('{}');\
                    const __loc = globalThis.__neo_location || globalThis.location;\
                    __loc.href = __u.href;\
                    __loc.protocol = __u.protocol;\
                    __loc.host = __u.host;\
                    __loc.hostname = __u.hostname;\
                    __loc.port = __u.port;\
                    __loc.pathname = __u.pathname;\
                    __loc.search = __u.search;\
                    __loc.hash = __u.hash;\
                    __loc.origin = __u.origin;\
                }}\
             }} catch(e) {{}}",
            escaped_url, escaped_url
        );
        self.runtime
            .execute_script("<set_location>", loc_js)
            .map_err(|e| RuntimeError::Dom(first_line(&e.to_string())))?;
        Ok(())
    }
}

// ─── JsRuntime trait implementation ───

impl JsRuntimeTrait for DenoRuntime {
    fn eval(&mut self, code: &str) -> Result<String, RuntimeError> {
        // Enter tokio runtime context so deno_core's WebTimers can create
        // tokio::time::Sleep futures when JS calls setTimeout/setInterval.
        let _guard = self.tokio_rt.enter();

        let wrapped = format!(
            "try {{ String(\n{}\n) }} catch(__e) {{ 'Error: ' + __e.message }}",
            code
        );

        let result = match self.runtime.execute_script("<eval>", wrapped.clone()) {
            Ok(r) => r,
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("erminated") {
                    // V8 termination flag still set — recover and retry once
                    self.runtime.v8_isolate().cancel_terminate_execution();
                    let _ = self
                        .runtime
                        .execute_script("<eval-recovery>", "void 0".to_string());
                    self.runtime
                        .execute_script("<eval-retry>", wrapped)
                        .map_err(|e2| RuntimeError::Eval(first_line(&e2.to_string())))?
                } else {
                    return Err(RuntimeError::Eval(first_line(&msg)));
                }
            }
        };

        let val = self.v8_value_to_string(result);
        self.drain_microtasks();
        Ok(val)
    }

    fn execute(&mut self, code: &str) -> Result<(), RuntimeError> {
        // Enter tokio runtime context so deno_core's WebTimers can create
        // tokio::time::Sleep futures when JS calls setTimeout/setInterval.
        let _guard = self.tokio_rt.enter();

        let wrapped = format!("try {{\n{}\n}} catch(__e) {{ if(typeof console!=='undefined')console.error('[script-error] ' + __e.message + ' @ ' + (__e.stack||'').split('\\n')[1]); }}", code);
        match self.runtime.execute_script("<script>", wrapped.clone()) {
            Ok(_) => {
                self.drain_microtasks();
                Ok(())
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("erminated") {
                    // V8 termination flag still set — recover and retry once
                    self.runtime.v8_isolate().cancel_terminate_execution();
                    let _ = self
                        .runtime
                        .execute_script("<exec-recovery>", "void 0".to_string());
                    self.runtime
                        .execute_script("<exec-retry>", wrapped)
                        .map_err(|e2| RuntimeError::Eval(first_line(&e2.to_string())))?;
                    Ok(())
                } else {
                    Err(RuntimeError::Eval(first_line(&msg)))
                }
            }
        }
    }

    fn load_module(&mut self, url: &str) -> Result<(), RuntimeError> {
        // Enter tokio runtime context — mod_evaluate runs top-level module code
        // which may call setTimeout/setInterval, triggering deno_core's WebTimers
        // that need tokio::time::sleep_until (requires active tokio context).
        let _guard = self.tokio_rt.enter();

        // Step 1: URL dedup
        if !self.module_evaluator.mark_url_loaded(url) {
            neo_trace!("[MODULE-LIFECYCLE] load_module SKIP (already loaded): {url}");
            return Ok(());
        }

        // Step 2: Parse specifier
        let specifier = deno_core::ModuleSpecifier::parse(url)
            .map_err(|e| RuntimeError::Module(e.to_string()))?;

        let load_start = Instant::now();
        neo_trace!("[MODULE-LIFECYCLE] load_module START: {url}");

        // Step 3: Notify JS
        self.notify_module_requested(url);

        let tb = get_trace_buffer(&self.runtime);

        // Step 4: Load (fetch + instantiate)
        tb.module_event(url, ModulePhase::Instantiate, None);
        let mod_id = match self.load_es_module(&specifier, url) {
            Ok(id) => id,
            Err(_) => {
                tb.module_event(url, ModulePhase::Error, Some("load_es_module failed"));
                return Ok(());  // non-fatal, already logged
            }
        };

        // Step 5: Track mod_id — returns PREVIOUS max for comparison
        let prev_max = self.module_evaluator.track_mod_id(mod_id);
        neo_trace!(
            "[MODULE-LIFECYCLE] load_module INSTANTIATED: {url} (id={mod_id}, prev_max={prev_max}, {}ms)",
            load_start.elapsed().as_millis()
        );

        // Step 6: Evaluate (with guards)
        if self.module_evaluator.should_evaluate(mod_id, prev_max) {
            self.evaluate_module(mod_id, url, load_start)?;
        } else {
            neo_trace!("[MODULE-LIFECYCLE] SKIP EVAL {url} (mod_id={mod_id}, prev_max={prev_max})");
        }

        // Step 7: Notify JS of completion
        self.notify_module_loaded(url);

        // Log module tracker state.
        neo_trace!(
            "[MODULE-LIFECYCLE] load_module DONE: {url} ({}ms) — tracker: pending={}, loaded={}, failed={}, total={}",
            load_start.elapsed().as_millis(),
            self.module_tracker.pending(),
            self.module_tracker.total_loaded(),
            self.module_tracker.total_failed(),
            self.module_tracker.total_requested(),
        );

        // kExplicit: drain microtasks after module evaluation so that
        // synchronous globalThis writes from module top-level code are
        // visible to subsequent eval() calls.
        self.drain_microtasks();

        Ok(())
    }

    fn pump_event_loop(&mut self) -> Result<bool, RuntimeError> {
        // Enter tokio context — microtask checkpoint may trigger timer creation.
        let _guard = self.tokio_rt.enter();

        // CRITICAL: Force V8 microtask checkpoint before running the event loop.
        // Without this, Promise.resolve().then() and async function bodies never
        // execute — their microtasks sit in V8's queue unprocessed.
        // Chromium does this automatically as part of its event loop cycle.
        {
            self.runtime.v8_isolate().perform_microtask_checkpoint();
        }

        let options = PollEventLoopOptions {
            wait_for_inspector: false,
        };
        let pump = EventLoopRunner::pump_once_with_options(
            &mut self.runtime,
            &self.tokio_rt,
            5,
            options,
        );
        match pump {
            PumpResult::Idle => Ok(true),
            PumpResult::Error(_) => Ok(false), // event loop error — treat as idle
            PumpResult::Timeout => Ok(true),   // timeout — there was work in progress
            PumpResult::Panic => {
                eprintln!("[neo-runtime] pump_event_loop: V8 PANIC caught — isolate may be corrupted");
                Ok(false)
            }
        }
    }

    fn run_until_settled(&mut self, timeout_ms: u64) -> Result<(), RuntimeError> {
        // Enter tokio context for the entire settle loop — event loop pumps
        // and microtask checkpoints may trigger WebTimer creation.
        let _guard = self.tokio_rt.enter();

        // NOTE: No V8 watchdog (terminate_execution). terminate_execution
        // permanently breaks V8's kAuto microtask auto-drain, making all
        // subsequent Promise.then() callbacks never execute. Confirmed via
        // systematic testing: httpbin (no terminate) → drain works,
        // ChatGPT (terminate fires) → drain permanently broken.
        // Instead, we rely on tokio::time::timeout to bound each event loop
        // poll iteration, and the hard deadline to exit the settle loop.

        let config = SettleConfig::bootstrap(timeout_ms);
        let module_tracker = &self.module_tracker;
        let mut runner = EventLoopRunner::new();

        let stats = runner.run_until_settled(
            &mut self.runtime,
            &self.tokio_rt,
            &config,
            || module_tracker.pending(),
        );

        // Map SettleReason to Result — preserve original behavior where
        // all exits returned Ok(()) (timeouts, errors were non-fatal).
        match stats.reason {
            SettleReason::Panic => {
                eprintln!("[neo-runtime] run_until_settled: V8 PANIC after {}ms — isolate may be corrupted", stats.elapsed_ms);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn run_until_interaction_stable(&mut self, timeout_ms: u64) -> Result<(), RuntimeError> {
        // Enter tokio context for the entire interaction-stable loop.
        let _guard = self.tokio_rt.enter();

        // NOTE: No V8 watchdog here either — terminate_execution permanently
        // breaks microtask auto-drain. Rely on tokio timeouts only.

        let config = SettleConfig::interaction(timeout_ms);
        let module_tracker = &self.module_tracker;
        let mut runner = EventLoopRunner::new();

        let stats = runner.run_until_interaction_stable(
            &mut self.runtime,
            &self.tokio_rt,
            &config,
            || module_tracker.pending(),
        );

        // Map SettleReason to Result — preserve original behavior where
        // all exits returned Ok(()) (timeouts, errors were non-fatal).
        match stats.reason {
            SettleReason::Panic => {
                eprintln!("[neo-runtime] run_until_interaction_stable: V8 PANIC after {}ms — isolate may be corrupted", stats.elapsed_ms);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn pending_tasks(&self) -> usize {
        self.tracker.pending()
    }

    fn eval_promise(&mut self, code: &str, timeout_ms: u64) -> Result<String, RuntimeError> {
        // Enter tokio context — execute_script may trigger WebTimer creation.
        let _guard = self.tokio_rt.enter();

        // Execute the code — which should return a Promise
        let global = self.runtime
            .execute_script("<eval-promise>", code.to_string())
            .map_err(|e| RuntimeError::Eval(first_line(&e.to_string())))?;

        match self.resolve_promise_value(global, timeout_ms) {
            Ok(val) => Ok(val),
            Err(PromiseResolveError::Runtime(msg)) => Err(RuntimeError::Eval(msg)),
            Err(PromiseResolveError::Eval(msg)) => Err(RuntimeError::Eval(msg)),
            Err(PromiseResolveError::Timeout(ms)) => Err(RuntimeError::Timeout { timeout_ms: ms, pending: 0 }),
            Err(PromiseResolveError::Panic) => {
                Err(RuntimeError::Eval("V8 panic during promise resolution".to_string()))
            }
        }
    }

    fn eval_and_settle(
        &mut self,
        code: &str,
        timeout_ms: u64,
    ) -> Result<EvalSettleResult, RuntimeError> {
        let start = Instant::now();
        let _guard = self.tokio_rt.enter();

        let global = self.eval_raw(code)?;

        if self.is_promise(&global) {
            // Resolve the Promise via disposable tokio runtime.
            let value = match self.resolve_promise_value(global, timeout_ms) {
                Ok(val) => val,
                Err(PromiseResolveError::Eval(msg)) => format!("Error: {msg}"),
                Err(PromiseResolveError::Timeout(_)) => "Error: timeout".to_string(),
                Err(PromiseResolveError::Panic) => {
                    eprintln!("[neo-runtime] eval_and_settle: PANIC caught during resolve_value");
                    "Error: V8 panic".to_string()
                }
                Err(PromiseResolveError::Runtime(msg)) => format!("Error: {msg}"),
            };

            Ok(self.settle_after_eval(start, timeout_ms, value))
        } else {
            // Not a Promise — return directly.
            let value = self.v8_value_ref_to_string(&global);

            Ok(EvalSettleResult {
                value,
                was_promise: false,
                settled_ms: start.elapsed().as_millis() as u64,
                pending_timers: 0,
            })
        }
    }

    fn reset_budgets(&mut self) {
        self.timer_budget.reset();
        self.fetch_budget.reset(); // Clear abort flag so app fetches get a fresh budget
        self.tracker.reset();
        self.module_tracker.reset();
        // Also reset JS-side callback budget and module counters
        let _ = self.execute("if(typeof __callbackCount!=='undefined'){__callbackCount=0;__budgetExhausted=false;}");
    }

    fn set_document_html(&mut self, html: &str, url: &str) -> Result<(), RuntimeError> {
        // Enter tokio runtime context — bootstrap.js and page scripts may call
        // setTimeout/setInterval which need the tokio reactor for WebTimers.
        let _guard = self.tokio_rt.enter();

        self.timer_budget.reset();
        self.tracker.reset();

        // R7d: Set page origin for module resolution.
        if let Ok(parsed) = url::Url::parse(url) {
            *self.page_origin.borrow_mut() = parsed.origin().ascii_serialization();
        }

        let escaped = Self::escape_html_for_js(html);
        let escaped_url = url.replace('\'', "\\'");

        // On first call, execute full bootstrap. On subsequent calls, just
        // reinitialize the DOM via __linkedom_parseHTML (legacy name, now uses happy-dom).
        // bootstrap.js uses const declarations which can't be re-executed in the same V8 context.
        let is_initialized = self
            .eval("typeof globalThis.__neo_initialized !== 'undefined' ? 'yes' : 'no'")
            .map(|v| v.contains("yes"))
            .unwrap_or(false);

        if is_initialized {
            self.reinit_document(&escaped, &escaped_url)?;
        } else {
            self.bootstrap_runtime(&escaped, &escaped_url)?;
        }

        self.set_location(&escaped_url)?;

        Ok(())
    }

    fn export_html(&mut self) -> Result<String, RuntimeError> {
        self.eval("globalThis.__neorender_export ? __neorender_export() : ''")
    }

    fn insert_module(&mut self, url: &str, source: &str) {
        self.store
            .borrow_mut()
            .scripts
            .insert(url.to_string(), source.to_string());
    }

    fn has_module(&self, url: &str) -> bool {
        self.store.borrow().scripts.contains_key(url)
    }

    fn mark_stub(&mut self, url: &str) {
        self.store.borrow_mut().stub_modules.insert(url.to_string());
    }

    fn get_module_source(&self, url: &str) -> Option<String> {
        self.store.borrow().scripts.get(url).cloned()
    }

    fn module_urls(&self) -> Vec<String> {
        self.store.borrow().scripts.keys().cloned().collect()
    }

    fn isolate_handle(&mut self) -> Option<RuntimeHandle> {
        let handle = self.runtime.v8_isolate().thread_safe_handle();
        Some(RuntimeHandle { inner: handle })
    }

    fn drain_trace_events(&mut self) -> Vec<crate::trace_events::TraceEvent> {
        let op_state = self.runtime.op_state();
        let state = op_state.borrow();
        if let Some(buf) = state.try_borrow::<crate::trace_events::TraceBuffer>() {
            buf.drain()
        } else {
            vec![]
        }
    }

    fn drain_navigation_requests(&mut self) -> Vec<String> {
        let op_state = self.runtime.op_state();
        let state = op_state.borrow();
        if let Some(queue) = state.try_borrow::<crate::ops::NavigationQueue>() {
            queue.drain()
        } else {
            vec![]
        }
    }

    fn get_cookies(&mut self) -> String {
        let op_state = self.runtime.op_state();
        let state = op_state.borrow();
        if let Some(cookies) = state.try_borrow::<crate::ops::CookieState>() {
            cookies.get_cookie_string()
        } else {
            String::new()
        }
    }

    fn set_cookie(&mut self, cookie_str: &str) {
        let op_state = self.runtime.op_state();
        let state = op_state.borrow();
        if let Some(cookies) = state.try_borrow::<crate::ops::CookieState>() {
            cookies.set_from_string(cookie_str);
        }
    }

    fn set_import_map(&mut self, map: crate::modules::ImportMap) {
        *self.import_map.borrow_mut() = Some(map);
    }

    fn reset_page_state(&mut self) {
        // Clear pre-fetched scripts (per-page artifacts).
        {
            let mut store = self.store.borrow_mut();
            store.scripts.clear();
            store.failed_urls.clear();
            store.stub_modules.clear();
        }
        // Clear import map (per-page).
        *self.import_map.borrow_mut() = None;
        // Reset scheduler budgets.
        self.timer_budget.reset();
        self.fetch_budget.reset();
        self.tracker.reset();
        self.module_tracker.reset();
        // Reset module evaluator (URLs, mod_ids, corruption flag).
        self.module_evaluator.reset();
        // Reset JS-side callback budget.
        let _ = self.execute(
            "if(typeof __callbackCount!=='undefined'){__callbackCount=0;__budgetExhausted=false;}",
        );
    }
}

// ─── Unit tests ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_line_single() {
        assert_eq!(first_line("hello"), "hello");
    }

    #[test]
    fn test_first_line_multi() {
        assert_eq!(first_line("first\nsecond\nthird"), "first");
    }

    #[test]
    fn test_first_line_empty() {
        assert_eq!(first_line(""), "");
    }

    #[test]
    fn test_escape_html_for_js_backtick() {
        let escaped = DenoRuntime::escape_html_for_js("hello `world`");
        assert_eq!(escaped, "hello \\`world\\`");
    }

    #[test]
    fn test_escape_html_for_js_template_literal() {
        let escaped = DenoRuntime::escape_html_for_js("price is ${amount}");
        assert_eq!(escaped, "price is \\${amount}");
    }

    #[test]
    fn test_escape_html_for_js_backslash() {
        let escaped = DenoRuntime::escape_html_for_js("path\\to\\file");
        assert_eq!(escaped, "path\\\\to\\\\file");
    }

    #[test]
    fn test_escape_html_for_js_combined() {
        let escaped = DenoRuntime::escape_html_for_js("`${x}\\n`");
        assert_eq!(escaped, "\\`\\${x}\\\\n\\`");
    }
}
