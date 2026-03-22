//! JsRuntime trait implementation for DenoRuntime.

use deno_core::PollEventLoopOptions;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::neo_trace;
use crate::v8::DenoRuntime;
use crate::{EvalSettleResult, JsRuntime as JsRuntimeTrait, RuntimeError, RuntimeHandle};

/// Quiescence signals reported by the JS `__neo_quiescence()` function.
///
/// Used by both `run_until_settled` and `run_until_interaction_stable` to
/// determine when the page has finished all async work and DOM mutations.
#[derive(serde::Deserialize, Default)]
struct Quiescence {
    #[serde(default)]
    idle_ms: u64,
    #[serde(default)]
    pending_timers: usize,
    #[serde(default)]
    pending_fetches: usize,
    #[serde(default)]
    pending_modules: usize,
    #[serde(default)]
    dom_mutations: usize,
}

/// Extract the first line of an error message.
pub(crate) fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).to_string()
}

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

        let scope = &mut self.runtime.handle_scope();
        let local = deno_core::v8::Local::new(scope, result);
        if let Some(s) = local.to_string(scope) {
            Ok(s.to_rust_string_lossy(scope))
        } else {
            Ok("undefined".to_string())
        }
    }

    fn execute(&mut self, code: &str) -> Result<(), RuntimeError> {
        // Enter tokio runtime context so deno_core's WebTimers can create
        // tokio::time::Sleep futures when JS calls setTimeout/setInterval.
        let _guard = self.tokio_rt.enter();

        let wrapped = format!("try {{\n{}\n}} catch(__e) {{ if(typeof console!=='undefined')console.error('[script-error] ' + __e.message + ' @ ' + (__e.stack||'').split('\\n')[1]); }}", code);
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

        let load_start = Instant::now();
        neo_trace!("[MODULE-LIFECYCLE] load_module START: {url}");

        // Notify JS side that a module load is starting.
        let escaped_url = url.replace('\'', "\\'");
        let _ = self.runtime.execute_script(
            "<module-track-req>",
            format!("typeof __neo_moduleRequested==='function'&&__neo_moduleRequested('{escaped_url}')"),
        );

        self.tokio_rt.block_on(async {
            // Phase 1: Load (fetch + instantiate)
            let mod_id = self
                .runtime
                .load_main_es_module(&specifier)
                .await
                .map_err(|e| {
                    neo_trace!("[MODULE-LIFECYCLE] load_module LOAD-FAILED: {url} — {}", first_line(&e.to_string()));
                    // Notify JS side of failure.
                    let _ = self.runtime.execute_script(
                        "<module-track-fail>",
                        format!("typeof __neo_moduleFailed==='function'&&__neo_moduleFailed('{escaped_url}','load-failed')"),
                    );
                    RuntimeError::Module(first_line(&e.to_string()))
                })?;

            neo_trace!(
                "[MODULE-LIFECYCLE] load_module INSTANTIATED: {url} (id={mod_id}, {}ms)",
                load_start.elapsed().as_millis()
            );

            // Phase 2: Evaluate
            let eval = self.runtime.mod_evaluate(mod_id);

            // Run event loop with timeout — modules that create infinite
            // timer loops (MobX, React scheduler) must not hang forever.
            match tokio::time::timeout(
                Duration::from_millis(5000),
                self.runtime.run_event_loop(PollEventLoopOptions::default()),
            )
            .await
            {
                Ok(Ok(())) => {
                    neo_trace!(
                        "[MODULE-LIFECYCLE] load_module EVENT-LOOP-IDLE: {url} ({}ms)",
                        load_start.elapsed().as_millis()
                    );
                }
                Ok(Err(e)) => {
                    neo_trace!(
                        "[MODULE-LIFECYCLE] load_module EVENT-LOOP-ERROR: {url} — {}",
                        first_line(&e.to_string())
                    );
                    let _ = self.runtime.execute_script(
                        "<module-track-fail>",
                        format!("typeof __neo_moduleFailed==='function'&&__neo_moduleFailed('{escaped_url}','event-loop-error')"),
                    );
                    return Err(RuntimeError::Module(format!(
                        "event loop: {}",
                        first_line(&e.to_string())
                    )));
                }
                Err(_) => {
                    // Timeout — module may have created timer loops.
                    // Don't fail — the module may have partially evaluated.
                    eprintln!("[MODULE] event loop timeout for {url} (5s) — continuing");
                    neo_trace!("[MODULE-LIFECYCLE] load_module EVENT-LOOP-TIMEOUT: {url} (5000ms)");
                }
            }

            // Phase 3: Await evaluation promise
            match tokio::time::timeout(Duration::from_millis(1000), eval).await {
                Ok(Ok(())) => {
                    neo_trace!(
                        "[MODULE-LIFECYCLE] load_module EVALUATED: {url} ({}ms total)",
                        load_start.elapsed().as_millis()
                    );
                }
                Ok(Err(e)) => {
                    neo_trace!(
                        "[MODULE-LIFECYCLE] load_module EVAL-FAILED: {url} — {}",
                        first_line(&e.to_string())
                    );
                    let _ = self.runtime.execute_script(
                        "<module-track-fail>",
                        format!("typeof __neo_moduleFailed==='function'&&__neo_moduleFailed('{escaped_url}','eval-failed')"),
                    );
                    return Err(RuntimeError::Module(first_line(&e.to_string())));
                }
                Err(_) => {
                    eprintln!("[MODULE] eval timeout for {url} (1s) — continuing");
                    neo_trace!("[MODULE-LIFECYCLE] load_module EVAL-TIMEOUT: {url} (1000ms)");
                }
            }

            // Notify JS side of success.
            let _ = self.runtime.execute_script(
                "<module-track-ok>",
                format!("typeof __neo_moduleLoaded==='function'&&__neo_moduleLoaded('{escaped_url}')"),
            );

            // Log module tracker state after this module completes.
            let tracker = &self.module_tracker;
            neo_trace!(
                "[MODULE-LIFECYCLE] load_module DONE: {url} ({}ms) — tracker: pending={}, loaded={}, failed={}, total={}",
                load_start.elapsed().as_millis(),
                tracker.pending(),
                tracker.total_loaded(),
                tracker.total_failed(),
                tracker.total_requested(),
            );

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
        let settle_start = Instant::now();

        // V8 watchdog: terminate_execution after deadline.
        let isolate_handle = self.runtime.v8_isolate().thread_safe_handle();
        let watchdog_deadline = deadline;
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel_flag.clone();
        let watchdog = std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(50));
                if cancel_clone.load(Ordering::Relaxed) {
                    return;
                }
                if Instant::now() >= watchdog_deadline {
                    isolate_handle.terminate_execution();
                    return;
                }
            }
        });

        // Quiescence parameters
        let min_settle_ms: u64 = 1500.min(timeout_ms * 2 / 3); // at most 2/3 of budget
        const QUIET_WINDOW_MS: u64 = 400; // no activity for this long = quiet
        const CHECK_INTERVAL_MS: u64 = 100; // how often to poll quiescence

        let result = self.tokio_rt.block_on(async {
            loop {
                // Hard deadline
                if Instant::now() >= deadline {
                    return Ok(());
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                let loop_timeout = Duration::from_millis(CHECK_INTERVAL_MS).min(remaining);
                if loop_timeout.is_zero() {
                    return Ok(());
                }

                // Drive the event loop
                match tokio::time::timeout(
                    loop_timeout,
                    self.runtime.run_event_loop(PollEventLoopOptions::default()),
                )
                .await
                {
                    Ok(Ok(())) => {
                        // Event loop went idle. Check quiescence via JS.
                    }
                    Ok(Err(e)) => {
                        eprintln!(
                            "[neo-runtime] event loop error (non-fatal): {}",
                            first_line(&e.to_string())
                        );
                        return Ok(());
                    }
                    Err(_) => {
                        // Timeout — event loop had work, loop again
                        continue;
                    }
                }

                // Query quiescence state from JS
                let quiescence = {
                    let code = "typeof __neo_quiescence==='function'?__neo_quiescence():'{}'";
                    match self.runtime.execute_script("<quiescence>", code.to_string()) {
                        Ok(val) => {
                            let scope = &mut self.runtime.handle_scope();
                            let local = deno_core::v8::Local::new(scope, val);
                            local.to_string(scope)
                                .map(|s| s.to_rust_string_lossy(scope))
                                .unwrap_or_default()
                        }
                        Err(_) => "{}".to_string(),
                    }
                };

                let q: Quiescence = serde_json::from_str(&quiescence).unwrap_or_default();

                let elapsed = settle_start.elapsed().as_millis() as u64;

                // Reset mutation counter after reading
                let _ = self.runtime.execute_script(
                    "<reset-mutations>",
                    "typeof __neo_resetMutationCount==='function'&&__neo_resetMutationCount()".to_string(),
                );

                // Also check Rust-side module tracker — catches modules that
                // are in-flight at the loader level (fetching, not yet evaluated).
                let rust_pending_modules = self.module_tracker.pending();

                // Quiescence criteria:
                // 1. Minimum settle time elapsed
                // 2. No recent JS activity (quiet window)
                // 3. No pending async work (JS-side AND Rust-side)
                // 4. No recent DOM mutations
                let min_elapsed = elapsed >= min_settle_ms;
                let quiet = q.idle_ms >= QUIET_WINDOW_MS;
                let no_pending = q.pending_timers == 0
                    && q.pending_fetches == 0
                    && q.pending_modules == 0
                    && rust_pending_modules == 0;
                let no_mutations = q.dom_mutations == 0;

                neo_trace!(
                    "[SETTLE] elapsed={}ms idle={}ms timers={} fetches={} js_modules={} rust_modules={} mutations={} -> {}",
                    elapsed, q.idle_ms, q.pending_timers, q.pending_fetches,
                    q.pending_modules, rust_pending_modules, q.dom_mutations,
                    if min_elapsed && quiet && no_pending && no_mutations { "SETTLED" } else { "waiting" }
                );

                if min_elapsed && quiet && no_pending && no_mutations {
                    return Ok(());
                }

                // Brief sleep before next check
                tokio::time::sleep(Duration::from_millis(50)).await;
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

    fn run_until_interaction_stable(&mut self, timeout_ms: u64) -> Result<(), RuntimeError> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let settle_start = Instant::now();

        // V8 watchdog: terminate_execution after deadline.
        let isolate_handle = self.runtime.v8_isolate().thread_safe_handle();
        let watchdog_deadline = deadline;
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel_flag.clone();
        let watchdog = std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(50));
                if cancel_clone.load(Ordering::Relaxed) {
                    return;
                }
                if Instant::now() >= watchdog_deadline {
                    isolate_handle.terminate_execution();
                    return;
                }
            }
        });

        // Interaction settle parameters — much more aggressive than bootstrap
        let min_settle_ms: u64 = 75_u64.min(timeout_ms * 2 / 3);
        const QUIET_WINDOW_MS: u64 = 400;
        const CHECK_INTERVAL_MS: u64 = 50;

        // Epoch tracking: after a DOM mutation or fetch resolve, we need at least
        // one more quiet check cycle before declaring settled. This prevents
        // cutting between React commit phases (e.g. setState -> render -> commit).
        let mut epoch_dirty = false; // true if we saw activity since last quiet check
        let mut saw_quiet_after_epoch = false; // true if we've had a quiet cycle after activity

        let result = self.tokio_rt.block_on(async {
            loop {
                // Hard deadline
                if Instant::now() >= deadline {
                    let elapsed = settle_start.elapsed().as_millis() as u64;
                    let reason = if epoch_dirty {
                        "timeout_after_mutation"
                    } else {
                        "timeout_with_pending_fetch"
                    };
                    eprintln!(
                        "[neo-runtime] interaction settle: {} after {}ms",
                        reason, elapsed
                    );
                    return Ok(());
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                let loop_timeout = Duration::from_millis(CHECK_INTERVAL_MS).min(remaining);
                if loop_timeout.is_zero() {
                    return Ok(());
                }

                // Drive the event loop
                match tokio::time::timeout(
                    loop_timeout,
                    self.runtime.run_event_loop(PollEventLoopOptions::default()),
                )
                .await
                {
                    Ok(Ok(())) => {
                        // Event loop went idle. Check quiescence via JS.
                    }
                    Ok(Err(e)) => {
                        eprintln!(
                            "[neo-runtime] interaction event loop error (non-fatal): {}",
                            first_line(&e.to_string())
                        );
                        return Ok(());
                    }
                    Err(_) => {
                        // Timeout — event loop had work, mark epoch dirty
                        epoch_dirty = true;
                        saw_quiet_after_epoch = false;
                        continue;
                    }
                }

                // Query quiescence state from JS
                let quiescence = {
                    let code = "typeof __neo_quiescence==='function'?__neo_quiescence():'{}'";
                    match self.runtime.execute_script("<quiescence-interaction>", code.to_string()) {
                        Ok(val) => {
                            let scope = &mut self.runtime.handle_scope();
                            let local = deno_core::v8::Local::new(scope, val);
                            local.to_string(scope)
                                .map(|s| s.to_rust_string_lossy(scope))
                                .unwrap_or_default()
                        }
                        Err(_) => "{}".to_string(),
                    }
                };

                let q: Quiescence = serde_json::from_str(&quiescence).unwrap_or_default();

                let elapsed = settle_start.elapsed().as_millis() as u64;

                // Reset mutation counter after reading
                let _ = self.runtime.execute_script(
                    "<reset-mutations-interaction>",
                    "typeof __neo_resetMutationCount==='function'&&__neo_resetMutationCount()".to_string(),
                );

                let rust_pending_modules = self.module_tracker.pending();

                // Detect activity in this cycle
                let has_activity = q.dom_mutations > 0 || q.pending_fetches > 0;
                let no_pending = q.pending_timers == 0
                    && q.pending_fetches == 0
                    && q.pending_modules == 0
                    && rust_pending_modules == 0;
                let no_mutations = q.dom_mutations == 0;
                let quiet = q.idle_ms >= QUIET_WINDOW_MS;
                let min_elapsed = elapsed >= min_settle_ms;

                // Epoch tracking: if we see activity, mark dirty and require
                // at least one more quiet cycle after it
                if has_activity {
                    epoch_dirty = true;
                    saw_quiet_after_epoch = false;
                } else if epoch_dirty && no_pending && no_mutations && quiet {
                    // First quiet cycle after activity — mark it but don't settle yet
                    saw_quiet_after_epoch = true;
                }

                // Settle criteria for interaction:
                // 1. min_settle elapsed (75ms)
                // 2. quiet window met
                // 3. no pending async work
                // 4. no recent DOM mutations
                // 5. If there was any epoch of activity, we need at least one
                //    quiet cycle AFTER it (saw_quiet_after_epoch)
                let epoch_ok = !epoch_dirty || saw_quiet_after_epoch;

                let settle_reason = if min_elapsed && quiet && no_pending && no_mutations && epoch_ok {
                    Some("quiet_no_pending")
                } else {
                    None
                };

                neo_trace!(
                    "[INTERACTION-SETTLE] elapsed={}ms idle={}ms timers={} fetches={} mutations={} epoch_dirty={} saw_quiet={} -> {}",
                    elapsed, q.idle_ms, q.pending_timers, q.pending_fetches,
                    q.dom_mutations, epoch_dirty, saw_quiet_after_epoch,
                    settle_reason.unwrap_or("waiting")
                );

                if let Some(reason) = settle_reason {
                    eprintln!(
                        "[neo-runtime] interaction settle: {} after {}ms",
                        reason, elapsed
                    );
                    return Ok(());
                }

                // Brief sleep before next check
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        });

        // Signal watchdog to stop
        cancel_flag.store(true, Ordering::Relaxed);
        let _ = watchdog.join();

        // Cancel any pending termination so future eval/execute calls work.
        self.runtime.v8_isolate().cancel_terminate_execution();

        // Verify the runtime is usable after potential termination.
        match self
            .runtime
            .execute_script("<interaction-settle-recovery>", "void 0".to_string())
        {
            Ok(_) => {}
            Err(_) => {
                self.runtime.v8_isolate().cancel_terminate_execution();
                let _ = self
                    .runtime
                    .execute_script("<interaction-settle-recovery2>", "void 0".to_string());
            }
        }

        result
    }

    fn pending_tasks(&self) -> usize {
        self.tracker.pending()
    }

    fn eval_promise(&mut self, code: &str, timeout_ms: u64) -> Result<String, RuntimeError> {
        // Execute the code — which should return a Promise
        let global = self.runtime
            .execute_script("<eval-promise>", code.to_string())
            .map_err(|e| RuntimeError::Eval(first_line(&e.to_string())))?;

        // Drive the event loop to resolve the promise using deno_core's
        // with_event_loop_promise / resolve_value. This is the correct way
        // to wait for a JS promise — it polls the event loop alongside
        // the promise, handling async ops, timers, and module evaluations.
        let result = self.tokio_rt.block_on(async {
            tokio::time::timeout(
                Duration::from_millis(timeout_ms),
                #[allow(deprecated)]
                self.runtime.resolve_value(global),
            )
            .await
        });

        match result {
            Ok(Ok(resolved)) => {
                let scope = &mut self.runtime.handle_scope();
                let local = deno_core::v8::Local::new(scope, resolved);
                if let Some(s) = local.to_string(scope) {
                    Ok(s.to_rust_string_lossy(scope))
                } else {
                    Ok("undefined".to_string())
                }
            }
            Ok(Err(e)) => Err(RuntimeError::Eval(first_line(&e.to_string()))),
            Err(_) => Err(RuntimeError::Timeout { timeout_ms, pending: 0 }),
        }
    }

    fn eval_and_settle(
        &mut self,
        code: &str,
        timeout_ms: u64,
    ) -> Result<EvalSettleResult, RuntimeError> {
        let start = Instant::now();
        let _guard = self.tokio_rt.enter();

        // Step 1: Execute the code
        let wrapped = format!(
            "try {{ (\n{}\n) }} catch(__e) {{ 'Error: ' + __e.message }}",
            code
        );
        let global = self
            .runtime
            .execute_script("<eval-settle>", wrapped)
            .map_err(|e| RuntimeError::Eval(first_line(&e.to_string())))?;

        // Step 2: Check if result is a Promise
        let is_promise = {
            let scope = &mut self.runtime.handle_scope();
            let local = deno_core::v8::Local::new(scope, &global);
            local.is_promise()
        };

        if is_promise {
            // Step 3a: Resolve the Promise with event loop driving
            #[allow(deprecated)]
            let result = self.tokio_rt.block_on(async {
                tokio::time::timeout(
                    Duration::from_millis(timeout_ms),
                    self.runtime.resolve_value(global),
                )
                .await
            });

            let value = match result {
                Ok(Ok(resolved)) => {
                    let scope = &mut self.runtime.handle_scope();
                    let local = deno_core::v8::Local::new(scope, resolved);
                    local
                        .to_string(scope)
                        .map(|s| s.to_rust_string_lossy(scope))
                        .unwrap_or_else(|| "undefined".to_string())
                }
                Ok(Err(e)) => format!("Error: {}", first_line(&e.to_string())),
                Err(_) => "Error: timeout".to_string(),
            };

            // Step 4: Brief settle after Promise resolution
            let remaining = timeout_ms.saturating_sub(start.elapsed().as_millis() as u64);
            if remaining > 100 {
                let _ = self.run_until_settled(remaining.min(1000));
            }

            let pending = self
                .eval("typeof __neo_pendingTimers==='function'?__neo_pendingTimers():0")
                .unwrap_or_default()
                .trim()
                .parse::<usize>()
                .unwrap_or(0);

            Ok(EvalSettleResult {
                value,
                was_promise: true,
                settled_ms: start.elapsed().as_millis() as u64,
                pending_timers: pending,
            })
        } else {
            // Step 3b: Not a Promise — return directly, brief pump
            let value = {
                let scope = &mut self.runtime.handle_scope();
                let local = deno_core::v8::Local::new(scope, global);
                local
                    .to_string(scope)
                    .map(|s| s.to_rust_string_lossy(scope))
                    .unwrap_or_else(|| "undefined".to_string())
            };

            // Brief pump for any side effects
            for _ in 0..10 {
                if let Ok(false) = self.pump_event_loop() {
                    break;
                }
            }

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

        let escaped = html
            .replace('\\', "\\\\")
            .replace('`', "\\`")
            .replace("${", "\\${");
        let escaped_url = url.replace('\'', "\\'");

        // On first call, execute full bootstrap. On subsequent calls, just
        // reinitialize the DOM via __linkedom_parseHTML (legacy name, now uses happy-dom).
        // bootstrap.js uses const declarations which can't be re-executed in the same V8 context.
        let is_first = self
            .eval("typeof globalThis.__neo_initialized !== 'undefined' ? 'yes' : 'no'")
            .map(|v| v.contains("yes"))
            .unwrap_or(false);

        if is_first {
            // Re-init: parse new HTML via happy-dom and replace document content.
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
