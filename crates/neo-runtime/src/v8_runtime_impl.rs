//! JsRuntime trait implementation for DenoRuntime.

use deno_core::PollEventLoopOptions;
use std::time::{Duration, Instant};

use crate::v8::DenoRuntime;
use crate::{JsRuntime as JsRuntimeTrait, RuntimeError};

/// Extract the first line of an error message.
pub(crate) fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).to_string()
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
        let wrapped = format!("try {{ {} }} catch(__e) {{ /* non-fatal */ }}", code);
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
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);

        self.tokio_rt.block_on(async {
            loop {
                let loop_timeout = Duration::from_millis(100)
                    .min(deadline.saturating_duration_since(Instant::now()));

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
        })
    }

    fn pending_tasks(&self) -> usize {
        self.tracker.pending()
    }

    fn set_document_html(&mut self, html: &str, url: &str) -> Result<(), RuntimeError> {
        self.timer_budget.reset();
        self.tracker.reset();

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

        let bootstrap_js: &str = include_str!("../../../js/bootstrap.js");
        self.runtime
            .execute_script("<neorender:bootstrap>", bootstrap_js.to_string())
            .map_err(|e| RuntimeError::Dom(format!("bootstrap: {}", first_line(&e.to_string()))))?;

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
