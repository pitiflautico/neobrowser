//! Sync wrapper for ChromeSession — runs CDP in a dedicated thread.
//!
//! Solves the problem of neo-chrome WebSocket hanging when called from
//! a context where another tokio runtime (e.g. V8's) is active.
//! The dedicated thread owns its own tokio runtime.

use crate::{ChromeError, Result};
use serde_json::Value;
use std::sync::{mpsc, Arc, Mutex};

/// A synchronous Chrome session. Internally runs async CDP on a background thread.
pub struct SyncChromeSession {
    /// Channel to send commands to the background thread.
    cmd_tx: mpsc::Sender<Command>,
    /// Channel to receive responses from the background thread.
    resp_rx: mpsc::Receiver<Response>,
    /// Join handle for the background thread.
    _thread: std::thread::JoinHandle<()>,
}

enum Command {
    Navigate(String),
    Eval(String),
    InjectScript(String),
    Close,
}

enum Response {
    Ok(Value),
    OkString(String),
    Error(String),
    Closed,
}

impl SyncChromeSession {
    /// Launch Chrome in a dedicated thread. Returns immediately.
    /// The Chrome process and CDP connection live on the background thread.
    pub fn launch(headless: bool) -> Result<Self> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
        let (resp_tx, resp_rx) = mpsc::channel::<Response>();

        let thread = std::thread::spawn(move || {
            // Own tokio runtime — completely isolated from any other runtime
            // multi_thread needed for tokio_tungstenite WebSocket send/recv loops
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("tokio runtime");

            rt.block_on(async {
                // Launch Chrome
                eprintln!("[sync-session] launching Chrome...");
                let session = match crate::session::ChromeSession::launch(None, headless).await {
                    Ok(s) => {
                        eprintln!("[sync-session] Chrome session created OK");
                        s
                    }
                    Err(e) => {
                        eprintln!("[sync-session] Chrome launch failed: {e}");
                        let _ = resp_tx.send(Response::Error(format!("Launch failed: {e}")));
                        return;
                    }
                };

                let session = Arc::new(Mutex::new(session));

                // Signal ready
                let _ = resp_tx.send(Response::Ok(serde_json::json!({"status": "ready"})));

                // Command loop
                while let Ok(cmd) = cmd_rx.recv() {
                    let mut s = session.lock().unwrap();
                    match cmd {
                        Command::Navigate(url) => {
                            match s.navigate(&url).await {
                                Ok(result) => {
                                    let _ = resp_tx.send(Response::Ok(
                                        serde_json::to_value(&result).unwrap_or_default()
                                    ));
                                }
                                Err(e) => {
                                    let _ = resp_tx.send(Response::Error(e.to_string()));
                                }
                            }
                        }
                        Command::Eval(js) => {
                            match s.eval(&js).await {
                                Ok(val) => {
                                    let _ = resp_tx.send(Response::OkString(val));
                                }
                                Err(e) => {
                                    let _ = resp_tx.send(Response::Error(e.to_string()));
                                }
                            }
                        }
                        Command::InjectScript(js) => {
                            match s.cdp.send_to(
                                &s.page_session_id,
                                "Page.addScriptToEvaluateOnNewDocument",
                                Some(serde_json::json!({"source": js})),
                            ).await {
                                Ok(_) => {
                                    let _ = resp_tx.send(Response::Ok(serde_json::json!({"injected": true})));
                                }
                                Err(e) => {
                                    let _ = resp_tx.send(Response::Error(e.to_string()));
                                }
                            }
                        }
                        Command::Close => {
                            drop(s);
                            let s = Arc::try_unwrap(session).ok();
                            if let Some(s) = s {
                                s.into_inner().unwrap().close().await;
                            }
                            let _ = resp_tx.send(Response::Closed);
                            return;
                        }
                    }
                }
            });
        });

        // Wait for ready signal
        match resp_rx.recv() {
            Ok(Response::Ok(_)) => {}
            Ok(Response::Error(e)) => return Err(ChromeError::ConnectionFailed(e)),
            _ => return Err(ChromeError::ConnectionFailed("Thread died".into())),
        }

        Ok(Self {
            cmd_tx,
            resp_rx,
            _thread: thread,
        })
    }

    /// Navigate to a URL. Blocks until page is loaded.
    pub fn navigate(&self, url: &str) -> Result<Value> {
        self.cmd_tx.send(Command::Navigate(url.to_string()))
            .map_err(|_| ChromeError::ConnectionFailed("Thread dead".into()))?;
        self.recv_value()
    }

    /// Evaluate JavaScript. Blocks until result is available.
    pub fn eval(&self, js: &str) -> Result<String> {
        self.cmd_tx.send(Command::Eval(js.to_string()))
            .map_err(|_| ChromeError::ConnectionFailed("Thread dead".into()))?;
        match self.resp_rx.recv() {
            Ok(Response::OkString(s)) => Ok(s),
            Ok(Response::Error(e)) => Err(ChromeError::CommandFailed { method: "eval".into(), error: e }),
            _ => Err(ChromeError::ConnectionFailed("Unexpected response".into())),
        }
    }

    /// Inject JavaScript to run before every page load (neomode patches).
    pub fn inject_script(&self, js: &str) -> Result<()> {
        self.cmd_tx.send(Command::InjectScript(js.to_string()))
            .map_err(|_| ChromeError::ConnectionFailed("Thread dead".into()))?;
        self.recv_value()?;
        Ok(())
    }

    /// Close Chrome and the background thread.
    pub fn close(self) {
        let _ = self.cmd_tx.send(Command::Close);
        let _ = self.resp_rx.recv(); // Wait for Closed
    }

    fn recv_value(&self) -> Result<Value> {
        match self.resp_rx.recv() {
            Ok(Response::Ok(v)) => Ok(v),
            Ok(Response::Error(e)) => Err(ChromeError::CommandFailed { method: "command".into(), error: e }),
            _ => Err(ChromeError::ConnectionFailed("Unexpected response".into())),
        }
    }
}

/// Neomode patches — 5 JS property overrides that make headless = real Chrome.
pub const NEOMODE_JS: &str = r#"
// Screen dimensions (headless defaults to 800x600)
Object.defineProperty(screen, 'width', {get: () => 1920});
Object.defineProperty(screen, 'height', {get: () => 1080});
Object.defineProperty(screen, 'availWidth', {get: () => 1920});
Object.defineProperty(screen, 'availHeight', {get: () => 1055});
Object.defineProperty(window, 'outerHeight', {get: () => 1055});
Object.defineProperty(window, 'innerHeight', {get: () => 968});

// WebDriver detection
Object.defineProperty(navigator, 'webdriver', {get: () => undefined});
delete navigator.__proto__.webdriver;

// Chrome runtime
window.chrome = window.chrome || {};
window.chrome.runtime = window.chrome.runtime || {connect:()=>{},sendMessage:()=>{}};

// Plugins (headless has 0)
if (navigator.plugins.length === 0) {
    Object.defineProperty(navigator, 'plugins', {
        get: () => {
            const p = [
                {name:'Chrome PDF Plugin',filename:'internal-pdf-viewer',description:'PDF'},
                {name:'Chrome PDF Viewer',filename:'mhjfbmdgcfjbbpaeojofohoefgiehjai',description:''},
                {name:'Native Client',filename:'internal-nacl-plugin',description:''},
            ];
            p.item = (i) => p[i]; p.namedItem = (n) => p.find(x=>x.name===n); p.refresh = ()=>{};
            return p;
        }
    });
}

// Languages
Object.defineProperty(navigator, 'languages', {get: () => ['en-US','en','es']});
Object.defineProperty(navigator, 'hardwareConcurrency', {get: () => 8});
Object.defineProperty(navigator, 'deviceMemory', {get: () => 8});

// Permissions
const origQuery = navigator.permissions?.query?.bind(navigator.permissions);
if (origQuery) {
    navigator.permissions.query = (p) => p.name === 'notifications'
        ? Promise.resolve({state: Notification.permission}) : origQuery(p);
}
"#;

/// Launch Chrome in neomode (headless + patches).
pub fn launch_neomode() -> Result<SyncChromeSession> {
    let session = SyncChromeSession::launch(true)?;
    session.inject_script(NEOMODE_JS)?;
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_neomode_js_has_all_patches() {
        assert!(NEOMODE_JS.contains("screen"));
        assert!(NEOMODE_JS.contains("1920"));
        assert!(NEOMODE_JS.contains("1080"));
        assert!(NEOMODE_JS.contains("outerHeight"));
        assert!(NEOMODE_JS.contains("innerHeight"));
        assert!(NEOMODE_JS.contains("968"));
    }
}
