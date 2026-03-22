//! JsRuntime trait implementation for DenoRuntime.

use deno_core::PollEventLoopOptions;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::v8::DenoRuntime;
use crate::{JsRuntime as JsRuntimeTrait, RuntimeError, RuntimeHandle};

/// Extract the first line of an error message.
pub(crate) fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).to_string()
}

impl JsRuntimeTrait for DenoRuntime {
    fn eval(&mut self, code: &str) -> Result<String, RuntimeError> {
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

        let scope = &mut self.runtime.handle_scope();
        let local = deno_core::v8::Local::new(scope, result);
        if let Some(s) = local.to_string(scope) {
            Ok(s.to_rust_string_lossy(scope))
        } else {
            Ok("undefined".to_string())
        }
    }

    fn execute(&mut self, code: &str) -> Result<(), RuntimeError> {
        let wrapped = format!("try {{\n{}\n}} catch(__e) {{ /* non-fatal */ }}", code);
        match self.runtime.execute_script("<script>", wrapped.clone()) {
            Ok(_) => Ok(()),
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
        let specifier = deno_core::ModuleSpecifier::parse(url)
            .map_err(|e| RuntimeError::Module(e.to_string()))?;

        self.tokio_rt.block_on(async {
            let mod_id = self
                .runtime
                .load_main_es_module(&specifier)
                .await
                .map_err(|e| RuntimeError::Module(first_line(&e.to_string())))?;

            let eval = self.runtime.mod_evaluate(mod_id);

            // Run event loop with timeout — modules that create infinite
            // timer loops (MobX, React scheduler) must not hang forever.
            match tokio::time::timeout(
                Duration::from_millis(5000),
                self.runtime.run_event_loop(PollEventLoopOptions::default()),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    return Err(RuntimeError::Module(format!(
                        "event loop: {}",
                        first_line(&e.to_string())
                    )));
                }
                Err(_) => {
                    // Timeout — module may have created timer loops.
                    // Don't fail — the module may have partially evaluated.
                    eprintln!("[MODULE] event loop timeout for {url} (5s) — continuing");
                }
            }

            // Try to get eval result, but don't block forever
            match tokio::time::timeout(Duration::from_millis(1000), eval).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    return Err(RuntimeError::Module(first_line(&e.to_string())));
                }
                Err(_) => {
                    eprintln!("[MODULE] eval timeout for {url} (1s) — continuing");
                }
            }

            Ok(())
        })
    }

    fn pump_event_loop(&mut self) -> Result<bool, RuntimeError> {
        self.tokio_rt.block_on(async {
            match tokio::time::timeout(
                Duration::from_millis(5),
                self.runtime
                    .run_event_loop(PollEventLoopOptions {
                        wait_for_inspector: false,
                        pump_v8_message_loop: true,
                    }),
            )
            .await
            {
                Ok(Ok(())) => Ok(true),
                Ok(Err(_)) => Ok(false), // event loop error — treat as idle
                Err(_) => Ok(true),      // timeout — there was work in progress
            }
        })
    }

    fn run_until_settled(&mut self, timeout_ms: u64) -> Result<(), RuntimeError> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);

        // V8 watchdog: terminate_execution after deadline.
        // tokio::timeout can't interrupt V8 microtask loops (they're synchronous
        // inside V8). Only terminate_execution can break infinite Promise.then chains.
        let isolate_handle = self.runtime.v8_isolate().thread_safe_handle();
        let watchdog_deadline = deadline;
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel_flag.clone();
        let watchdog = std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(50));
                if cancel_clone.load(Ordering::Relaxed) {
                    return; // Cancelled — event loop finished normally
                }
                if Instant::now() >= watchdog_deadline {
                    isolate_handle.terminate_execution();
                    return;
                }
            }
        });

        let result = self.tokio_rt.block_on(async {
            loop {
                // Hard deadline check BEFORE each iteration
                if Instant::now() >= deadline {
                    return Ok(());
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                let loop_timeout = Duration::from_millis(100).min(remaining);
                if loop_timeout.is_zero() {
                    return Ok(());
                }

                match tokio::time::timeout(
                    loop_timeout,
                    self.runtime.run_event_loop(PollEventLoopOptions::default()),
                )
                .await
                {
                    Ok(Ok(())) => {
                        if self.tracker.is_settled() {
                            return Ok(());
                        }
                    }
                    Ok(Err(e)) => {
                        eprintln!(
                            "[neo-runtime] event loop error (non-fatal): {}",
                            first_line(&e.to_string())
                        );
                        return Ok(());
                    }
                    Err(_) => {
                        // Timeout on this iteration — check overall deadline.
                    }
                }

                if Instant::now() >= deadline {
                    let pending = self.tracker.pending();
                    if pending > 0 {
                        return Err(RuntimeError::Timeout {
                            timeout_ms,
                            pending,
                        });
                    }
                    return Ok(());
                }
            }
        });

        // Signal watchdog to stop (prevents late termination if we finished early)
        cancel_flag.store(true, Ordering::Relaxed);
        let _ = watchdog.join();

        // Cancel any pending termination so future eval/execute calls work.
        self.runtime.v8_isolate().cancel_terminate_execution();

        // Verify the runtime is usable after potential termination.
        // If the watchdog fired, V8 may still have residual termination state.
        match self
            .runtime
            .execute_script("<settle-recovery>", "void 0".to_string())
        {
            Ok(_) => {} // Runtime recovered successfully
            Err(_) => {
                // Runtime still poisoned — cancel again and retry
                self.runtime.v8_isolate().cancel_terminate_execution();
                let _ = self
                    .runtime
                    .execute_script("<settle-recovery2>", "void 0".to_string());
            }
        }

        result
    }

    fn pending_tasks(&self) -> usize {
        self.tracker.pending()
    }

    fn set_document_html(&mut self, html: &str, url: &str) -> Result<(), RuntimeError> {
        self.timer_budget.reset();
        self.tracker.reset();

        // R7d: Set page origin for module resolution.
        if let Ok(parsed) = url::Url::parse(url) {
            *self.page_origin.borrow_mut() = parsed.origin().ascii_serialization();
        }

        let escaped = html
            .replace('\\', "\\\\")
            .replace('`', "\\`")
            .replace("${", "\\${");
        let escaped_url = url.replace('\'', "\\'");

        // On first call, execute full bootstrap. On subsequent calls, just
        // reinitialize the DOM via __linkedom_parseHTML (bootstrap.js uses
        // const declarations which can't be re-executed in the same V8 context).
        let is_first = self
            .eval("typeof globalThis.__neo_initialized !== 'undefined' ? 'yes' : 'no'")
            .map(|v| v.contains("yes"))
            .unwrap_or(false);

        if is_first {
            // Re-init: parse new HTML and replace document CONTENT (not the
            // document object itself — globalThis.document is non-replaceable
            // once linkedom initializes it).
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
        } else {
            // First time: set HTML and run full bootstrap + shim.
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

            // Mark as initialized.
            let _ = self.runtime.execute_script(
                "<mark_init>",
                "globalThis.__neo_initialized = true".to_string(),
            );
        }

        // Set location properties directly on __neo_location to avoid
        // triggering navigation interception from the browser shim.
        let loc_js = format!(
            "try {{\
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
}
