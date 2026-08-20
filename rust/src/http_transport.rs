//! MCP over Streamable HTTP, with authentication, origin validation and per-session
//! isolation.
//!
//! stdio remains the primary local transport and nothing here changes it. This exists
//! for the cases stdio cannot serve: a container, a remote dev box, several clients
//! against one host.
//!
//! It is off unless a port is configured, and when on it enforces three things the MCP
//! specification is explicit about, each of which is a real attack rather than a
//! checkbox:
//!
//! 1. **Authentication.** A bearer token on every request. Without it, anything that can
//!    reach the port drives a browser holding the user's sessions.
//! 2. **Origin validation.** A browser page can POST to `127.0.0.1` — and with DNS
//!    rebinding, a remote page can reach a LAN-bound port too. The `Origin` header is
//!    what distinguishes a real client from a page, and an unexpected one is rejected.
//! 3. **Session isolation.** Each `Mcp-Session-Id` gets its own [`crate::browser::Browser`], hence its
//!    own Chrome profile and its own cookies. Sharing one browser between callers would
//!    hand session A's logged-in state to session B.
//!
//! Binding defaults to loopback. A non-loopback bind is possible but requires an
//! explicit opt-in, because exposing this on a LAN interface is a materially different
//! decision from running it locally.
//!
//! Split into [`origin`] (the check that keeps a web page from driving the browser),
//! [`mod@serve`] (the listener and per-session isolation) and [`respond`] (hand-written
//! responses). The transport state and the token file live here.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::tools::{Registry, ToolCtx};

pub mod origin;
pub mod respond;
pub mod serve;

pub use origin::origin_allowed;
pub use serve::serve;

/// Maximum request body. A JSON-RPC call is small; unbounded reads are how a client
/// makes the server allocate until it dies.
const MAX_BODY: usize = 8 * 1024 * 1024;

/// Sessions idle longer than this are reaped, releasing their Chrome.
///
/// Necessary rather than tidy: every session owns a browser, so a client that
/// disconnects without cleanup would otherwise leak a Chrome per connection until the
/// host runs out of memory.
const SESSION_IDLE_SECS: u64 = 900;

/// One client session: its own browser, trace and policy.
struct Session {
    ctx: ToolCtx,
    last_seen: std::time::Instant,
}

pub struct HttpTransport {
    bind: String,
    port: u16,
    token: String,
    registry: Arc<Registry>,
    sessions: Mutex<HashMap<String, Session>>,
}

impl HttpTransport {
    pub fn new(bind: String, port: u16, registry: Arc<Registry>) -> Arc<Self> {
        Arc::new(Self {
            bind,
            port,
            token: crate::vault::random_token_hex(),
            registry,
            sessions: Mutex::new(HashMap::new()),
        })
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// Constant-time bearer comparison. Same reasoning as the bridge: a variable-time
    /// compare on a secret is a latent timing oracle.
    fn authorized(&self, header: Option<&str>) -> bool {
        let Some(value) = header else { return false };
        let presented = value
            .trim()
            .strip_prefix("Bearer ")
            .or_else(|| value.trim().strip_prefix("bearer "))
            .unwrap_or(value.trim());
        let (a, b) = (self.token.as_bytes(), presented.as_bytes());
        if a.len() != b.len() {
            return false;
        }
        a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }

    /// Get or create the session for `id`, and reap idle ones.
    async fn session_ctx(&self, id: &str) -> ToolCtx {
        let mut sessions = self.sessions.lock().await;

        // Reap first, so a long-running server does not accumulate browsers. Collected
        // then dropped outside the retain so each browser's shutdown is awaited.
        let stale: Vec<String> = sessions
            .iter()
            .filter(|(_, s)| s.last_seen.elapsed().as_secs() > SESSION_IDLE_SECS)
            .map(|(k, _)| k.clone())
            .collect();
        for key in stale {
            if let Some(s) = sessions.remove(&key) {
                tracing::info!(session = %key, "reaping an idle HTTP session");
                s.ctx.browser.shutdown().await;
            }
        }

        if let Some(s) = sessions.get_mut(id) {
            s.last_seen = std::time::Instant::now();
            return s.ctx.clone();
        }

        // A fresh session: its own browser on its own profile, so nothing is shared
        // with any other caller.
        let ctx = ToolCtx {
            browser: Arc::new(crate::browser::Browser::with_profile(id)),
            registry: self.registry.clone(),
            policy: Arc::new(crate::policy::Policy::from_env()),
            trace: Arc::new(crate::trace::Trace::new(format!("trace_http_{id}"))),
            // The bridge drives the user's own browser and is a single-user local
            // affair; exposing it through a network transport would mean one HTTP
            // client could drive another user's tabs.
            bridge: None,
        };
        tracing::info!(session = %id, "new HTTP session with an isolated profile");
        sessions.insert(
            id.to_string(),
            Session {
                ctx: ctx.clone(),
                last_seen: std::time::Instant::now(),
            },
        );
        ctx
    }

    async fn end_session(&self, id: &str) -> bool {
        let removed = self.sessions.lock().await.remove(id);
        match removed {
            Some(s) => {
                s.ctx.browser.shutdown().await;
                true
            }
            None => false,
        }
    }

    pub async fn session_count(&self) -> usize {
        self.sessions.lock().await.len()
    }
}

/// Where the HTTP bearer token is written, so `neobrowser http token` can print it.
pub fn token_path() -> std::path::PathBuf {
    crate::paths::home().join("http.token")
}

pub fn read_token_file() -> std::io::Result<String> {
    Ok(std::fs::read_to_string(token_path())?.trim().to_string())
}

/// The transport's configuration, or `None` when it is off (the default).
pub fn configured() -> Option<(String, u16)> {
    let port = std::env::var("NEOBROWSER_HTTP_PORT")
        .ok()
        .and_then(|v| v.trim().parse::<u16>().ok())
        .filter(|p| *p >= 1024)?;
    // Loopback unless explicitly overridden: binding a browser-driving API to every
    // interface should never be something that happens by default.
    let bind = std::env::var("NEOBROWSER_HTTP_BIND")
        .ok()
        .filter(|b| !b.trim().is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_string());
    Some((bind, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transport() -> Arc<HttpTransport> {
        HttpTransport::new(
            "127.0.0.1".into(),
            0,
            Arc::new(crate::tool_impls::build_registry()),
        )
    }

    #[test]
    fn bearer_tokens_are_long_random_and_constant_time_compared() {
        let a = transport();
        let b = transport();
        assert_ne!(a.token(), b.token());
        assert!(a.token().len() >= 64);
        assert!(a.authorized(Some(&format!("Bearer {}", a.token()))));
        // A bare token without the scheme is accepted too, since clients differ.
        assert!(a.authorized(Some(a.token())));
        assert!(!a.authorized(Some(&format!("Bearer {}", b.token()))));
        assert!(!a.authorized(None));
        assert!(!a.authorized(Some("Bearer short")));
    }

    /// A browser page cannot forge `Origin`, so validating it is what stops a page —
    /// or a DNS-rebound remote page — from driving this transport.
    #[test]
    fn only_loopback_origins_are_accepted() {
        assert!(origin_allowed(None), "a native client sends no Origin");
        assert!(origin_allowed(Some("http://127.0.0.1:3000")));
        assert!(origin_allowed(Some("http://localhost:5173")));
        assert!(origin_allowed(Some("http://[::1]:8080")));

        assert!(!origin_allowed(Some("https://evil.test")));
        assert!(!origin_allowed(Some("http://attacker.example.com")));
        // A sandboxed iframe or file:// page reports null — not a legitimate client.
        assert!(!origin_allowed(Some("null")));
        // The classic near-miss: a host that merely starts with the right characters.
        // A prefix check accepts these; only an exact host comparison rejects them.
        assert!(!origin_allowed(Some("http://localhost.evil.test")));
        assert!(!origin_allowed(Some("http://127.0.0.1.evil.test")));
        assert!(!origin_allowed(Some("http://localhostx")));
        // A non-http scheme is not a browser origin we should ever trust.
        assert!(!origin_allowed(Some("file://localhost")));
        // Unparseable garbage fails closed.
        assert!(!origin_allowed(Some("not a url at all")));
    }

    #[tokio::test]
    async fn each_session_id_gets_its_own_isolated_browser() {
        let t = transport();
        let a = t.session_ctx("client-a").await;
        let b = t.session_ctx("client-b").await;
        assert_eq!(t.session_count().await, 2);
        // Separate browser instances: sharing one would hand session A's cookies to B.
        assert!(
            !Arc::ptr_eq(&a.browser, &b.browser),
            "sessions must not share a browser"
        );
        // Separate traces, so one client's timeline is not polluted by another's.
        assert_ne!(a.trace.trace_id(), b.trace.trace_id());

        // The same id returns the same session rather than a fresh browser per request.
        let a2 = t.session_ctx("client-a").await;
        assert!(Arc::ptr_eq(&a.browser, &a2.browser));
        assert_eq!(t.session_count().await, 2);
    }

    #[tokio::test]
    async fn a_session_can_be_ended_and_ending_is_idempotent() {
        let t = transport();
        t.session_ctx("gone").await;
        assert!(t.end_session("gone").await, "first end reports it existed");
        assert!(
            !t.end_session("gone").await,
            "second end reports it did not"
        );
        assert_eq!(t.session_count().await, 0);
    }

    /// A session id arrives over the network and becomes a directory name. An
    /// unvalidated one would point Chrome's user-data dir wherever the caller likes.
    #[test]
    fn hostile_session_ids_cannot_escape_the_profiles_directory() {
        let base = crate::paths::profiles_base();
        for hostile in [
            "../../etc",
            "../../../.ssh",
            "..",
            ".",
            "/absolute/path",
            "with/slash",
            "",
            "....//....//",
        ] {
            let name = crate::paths::sanitize_profile_name(hostile);
            let dir = base.join(&name);
            assert!(
                dir.starts_with(&base),
                "{hostile:?} -> {name:?} escaped to {dir:?}"
            );
            assert!(
                !dir.components().any(|c| matches!(
                    c,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )),
                "{hostile:?} -> {name:?} produced a climbing path"
            );
            assert_eq!(
                dir.components().count(),
                base.components().count() + 1,
                "{hostile:?} -> {name:?} changed the depth"
            );
        }
    }

    #[test]
    fn the_transport_is_off_unless_a_valid_port_is_set() {
        let _g = crate::env_test_guard();
        let prev = std::env::var("NEOBROWSER_HTTP_PORT").ok();
        std::env::remove_var("NEOBROWSER_HTTP_PORT");
        assert_eq!(configured(), None, "must be opt-in");

        std::env::set_var("NEOBROWSER_HTTP_PORT", "8931");
        assert_eq!(configured(), Some(("127.0.0.1".into(), 8931)));

        // Loopback by default; a non-loopback bind must be spelled out.
        std::env::set_var("NEOBROWSER_HTTP_BIND", "0.0.0.0");
        assert_eq!(configured(), Some(("0.0.0.0".into(), 8931)));
        std::env::remove_var("NEOBROWSER_HTTP_BIND");

        // Privileged ports and nonsense do not silently become a default.
        std::env::set_var("NEOBROWSER_HTTP_PORT", "80");
        assert_eq!(configured(), None);
        std::env::set_var("NEOBROWSER_HTTP_PORT", "banana");
        assert_eq!(configured(), None);

        match prev {
            Some(v) => std::env::set_var("NEOBROWSER_HTTP_PORT", v),
            None => std::env::remove_var("NEOBROWSER_HTTP_PORT"),
        }
    }
}
