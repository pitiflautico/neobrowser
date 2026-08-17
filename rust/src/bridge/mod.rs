//! The agent half of the Chrome bridge: a localhost queue the extension polls.
//!
//! See `extension/README.md` for the security model. In short: instead of copying the
//! user's cookies into a second browser, or opening `--remote-debugging-port` on their
//! everyday Chrome and exposing every tab, the user shares one tab at a time from a
//! popup and can revoke it.
//!
//! The transport is a poll rather than a socket, and that is forced by the platform: a
//! Manifest V3 service worker is terminated when idle, so a long-lived WebSocket from
//! the extension would be killed with it. The extension asks for work; this side hands
//! out queued CDP commands and collects the results.
//!
//! Bound to `127.0.0.1` and **authenticated with a per-session token**.
//!
//! Loopback alone is not a boundary here, and it is worth being precise about why. Any
//! web page the user visits can issue a request to `http://127.0.0.1:<port>` — a
//! `text/plain` POST is a "simple" request, so there is no preflight, and while the
//! browser blocks the page from *reading* the response, the side effect still happens.
//! Without authentication a hostile page could therefore:
//!
//! * POST a forged result to `/bridge/results` and poison an in-flight CDP command,
//!   feeding the agent a fabricated answer; or
//! * POST to `/bridge` and drain the queue, so the real extension never receives the
//!   commands and every action times out.
//!
//! Two things close that. Every request must carry `X-NeoBrowser-Token`, which is a
//! *custom* header — that alone makes the request non-simple, so a page cannot send it
//! without a preflight this server refuses. And command ids are random rather than
//! sequential, so a forged result cannot address an outstanding waiter even if the
//! token were known.
//!
//! The token lives in `~/.neobrowser/bridge.token` at 0600, so a same-uid process can
//! read it — the same trust the MCP stdio transport already implies — while nothing
//! else can. Per-tab consent is still enforced in the extension, where Chrome shows
//! its own "being debugged" banner.
//!
//! Split so the listener sits apart from the queue it serves: [`server`] accepts
//! connections and authenticates them, while the queue, its pending map and the token file
//! stay here.
//!
//! Split so the queue sits apart from the listener that serves it: [`queue`] holds the
//! pending commands and their random ids, [`server`] accepts and authenticates connections,
//! and the token file stays here.

pub mod queue;
pub mod server;

pub use queue::Bridge;
pub use server::serve;

/// How long a queued command waits for the extension before it is abandoned.
///
/// Bounded because the extension may never come back — the browser was closed, the user
/// revoked the tab — and a caller blocked forever is worse than a caller told the
/// bridge is not answering.
const COMMAND_TIMEOUT_SECS: u64 = 20;

/// Header carrying the session token. Custom on purpose: a custom header cannot be set
/// on a cross-origin "simple" request, so requiring one blocks the browser-page vector
/// outright rather than relying on the token staying secret.
const TOKEN_HEADER: &str = "x-neobrowser-token";

/// Maximum request body the bridge will read.
///
/// A CDP result can be large (a DOM snapshot), but unbounded reads from a socket are
/// how a local process makes a server allocate until it dies.
const MAX_BODY: usize = 32 * 1024 * 1024;

/// Where the bridge token is written for the user to read.
pub fn token_path() -> std::path::PathBuf {
    crate::paths::home().join("bridge.token")
}

/// Read the token a previous or running session wrote.
pub fn read_token_file() -> std::io::Result<String> {
    Ok(std::fs::read_to_string(token_path())?.trim().to_string())
}

/// Bridge port from `NEOBROWSER_BRIDGE_PORT`, when the bridge is enabled at all.
pub fn configured_port() -> Option<u16> {
    std::env::var("NEOBROWSER_BRIDGE_PORT")
        .ok()
        .and_then(|v| v.trim().parse::<u16>().ok())
        .filter(|p| *p >= 1024)
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use tokio::net::TcpListener;

    use super::queue::Bridge;

    use super::server::{find_header_end, handle_connection};
    use super::*;

    #[test]
    fn header_end_is_found_only_on_a_complete_terminator() {
        // "GET / HTTP/1.1" is 14 bytes, so the terminator starts at 14 and the body
        // begins at 14 + 4 — which is what `handle_connection` relies on.
        let req = b"GET / HTTP/1.1\r\n\r\nbody";
        assert_eq!(find_header_end(req), Some(14));
        assert_eq!(&req[14 + 4..], b"body");
        // A partial terminator must not be treated as complete, or the body would be
        // read as headers.
        assert_eq!(find_header_end(b"GET / HTTP/1.1\r\n"), None);
        assert_eq!(find_header_end(b""), None);
    }

    #[tokio::test]
    async fn a_command_is_queued_and_handed_to_the_poller() {
        let bridge = Bridge::new(9999);
        // Mark connected, as a real poll would.
        bridge.take_work(vec![7]).await;
        assert!(bridge.is_connected().await);
        assert_eq!(bridge.shared_tabs().await, vec![7]);

        let b = bridge.clone();
        let handle = tokio::spawn(async move { b.send(7, "Page.enable", json!({})).await });

        // Give the sender a moment to enqueue, then poll as the extension does.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let work = bridge.take_work(vec![7]).await;
        let commands = work["commands"].as_array().unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0]["method"], "Page.enable");
        assert_eq!(commands[0]["tabId"], 7);
        let id = commands[0]["id"].as_u64().unwrap();

        bridge
            .deliver(&[json!({ "id": id, "result": { "ok": true } })])
            .await;
        let result = handle.await.unwrap().unwrap();
        assert_eq!(result["ok"], true);
    }

    #[tokio::test]
    async fn an_error_result_reaches_the_caller_as_an_error() {
        let bridge = Bridge::new(9999);
        bridge.take_work(vec![1]).await;
        let b = bridge.clone();
        let handle = tokio::spawn(async move { b.send(1, "X.y", json!({})).await });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let work = bridge.take_work(vec![1]).await;
        let id = work["commands"][0]["id"].as_u64().unwrap();
        bridge
            .deliver(&[json!({ "id": id, "error": "tab 1 is not shared" })])
            .await;
        let err = handle.await.unwrap().unwrap_err();
        assert!(err.contains("not shared"), "{err}");
    }

    /// Sending before the extension has ever polled must say so, rather than queueing
    /// into a void and timing out twenty seconds later.
    #[tokio::test]
    async fn sending_without_a_connected_extension_fails_fast() {
        let bridge = Bridge::new(9999);
        let err = bridge.send(1, "Page.enable", json!({})).await.unwrap_err();
        assert!(err.contains("has not connected"), "{err}");
        assert!(
            err.contains("chrome://extensions"),
            "must say how to fix it"
        );
    }

    /// A result arriving after its caller gave up must be dropped, not delivered to
    /// whatever is waiting next.
    #[tokio::test]
    async fn a_late_result_is_dropped_rather_than_misdelivered() {
        let bridge = Bridge::new(9999);
        bridge.take_work(vec![1]).await;
        // No waiter registered for id 42.
        let delivered = bridge
            .deliver(&[json!({ "id": 42, "result": { "stale": true } })])
            .await;
        assert_eq!(delivered, 0);
    }

    #[tokio::test]
    async fn results_without_an_id_are_ignored() {
        let bridge = Bridge::new(9999);
        let delivered = bridge
            .deliver(&[json!({ "result": {} }), json!("nonsense")])
            .await;
        assert_eq!(delivered, 0);
    }

    #[test]
    fn the_bridge_is_off_unless_a_valid_port_is_configured() {
        let _g = crate::env_test_guard();
        let prev = std::env::var("NEOBROWSER_BRIDGE_PORT").ok();
        std::env::remove_var("NEOBROWSER_BRIDGE_PORT");
        assert_eq!(configured_port(), None, "must be opt-in");
        std::env::set_var("NEOBROWSER_BRIDGE_PORT", "9333");
        assert_eq!(configured_port(), Some(9333));
        // A privileged port would need root, and nonsense must not silently become a
        // default port that the user did not choose.
        std::env::set_var("NEOBROWSER_BRIDGE_PORT", "80");
        assert_eq!(configured_port(), None);
        std::env::set_var("NEOBROWSER_BRIDGE_PORT", "banana");
        assert_eq!(configured_port(), None);
        match prev {
            Some(v) => std::env::set_var("NEOBROWSER_BRIDGE_PORT", v),
            None => std::env::remove_var("NEOBROWSER_BRIDGE_PORT"),
        }
    }

    /// End-to-end over a real socket: the wire format has to be right, not just the
    /// queue logic.
    #[tokio::test]
    async fn the_http_surface_speaks_the_protocol_the_extension_expects() {
        let bridge = Bridge::new(0);
        // Bind an ephemeral port ourselves so the test never collides with a real one.
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let b = bridge.clone();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let _ = handle_connection(&b, &mut socket).await;
            }
        });

        let client = reqwest::Client::new();
        let res = client
            .post(format!("http://127.0.0.1:{port}/bridge"))
            .header("X-NeoBrowser-Token", bridge.token())
            .json(&json!({ "shared_tabs": [3, 4] }))
            .send()
            .await
            .unwrap();
        assert!(res.status().is_success());
        let body: Value = res.json().await.unwrap();
        assert!(body["commands"].is_array());
        assert_eq!(bridge.shared_tabs().await, vec![3, 4]);
        assert!(bridge.is_connected().await);
    }

    /// The finding this closes: without a token, a web page the user is visiting could
    /// POST a forged result and poison an in-flight command.
    #[tokio::test]
    async fn an_unauthenticated_request_is_refused_before_the_body_is_parsed() {
        let bridge = Bridge::new(0);
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let b = bridge.clone();
        tokio::spawn(async move {
            for _ in 0..3 {
                if let Ok((mut socket, _)) = listener.accept().await {
                    let _ = handle_connection(&b, &mut socket).await;
                }
            }
        });
        let client = reqwest::Client::new();

        // No token at all.
        let res = client
            .post(format!("http://127.0.0.1:{port}/bridge/results"))
            .json(&json!({ "results": [{ "id": 1, "result": { "forged": true } }] }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status().as_u16(), 401);

        // A wrong token of the same length.
        let wrong = "0".repeat(bridge.token().len());
        let res = client
            .post(format!("http://127.0.0.1:{port}/bridge"))
            .header("X-NeoBrowser-Token", &wrong)
            .json(&json!({ "shared_tabs": [1] }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status().as_u16(), 401);

        // And the rejected polls must NOT have marked the bridge connected, or an
        // attacker could enable it and then drain the queue.
        assert!(!bridge.is_connected().await);
    }

    #[test]
    fn tokens_are_long_random_and_compared_in_constant_time() {
        let a = Bridge::new(1);
        let b = Bridge::new(1);
        assert_ne!(a.token(), b.token(), "tokens must not be predictable");
        assert!(
            a.token().len() >= 64,
            "token too short: {}",
            a.token().len()
        );
        assert!(a.token_matches(Some(a.token())));
        assert!(!a.token_matches(Some(b.token())));
        assert!(!a.token_matches(None));
        // Length mismatch must not match, and must not panic.
        assert!(!a.token_matches(Some("short")));
    }

    /// A sequential id lets a forged result address an outstanding command by guessing.
    #[tokio::test]
    async fn command_ids_are_not_guessable() {
        let bridge = Bridge::new(0);
        bridge.take_work(vec![1]).await;
        let mut ids = std::collections::HashSet::new();
        for _ in 0..5 {
            let b = bridge.clone();
            tokio::spawn(async move { b.send(1, "X.y", json!({})).await });
        }
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        let work = bridge.take_work(vec![1]).await;
        for cmd in work["commands"].as_array().unwrap() {
            let id = cmd["id"].as_u64().unwrap();
            assert!(id > 1000, "id {id} looks sequential");
            assert!(ids.insert(id), "duplicate id {id}");
        }
        assert_eq!(ids.len(), 5);
    }

    #[tokio::test]
    async fn unknown_routes_and_methods_are_refused() {
        let bridge = Bridge::new(0);
        for (path, method, expect) in [("/nope", "POST", 404u16), ("/bridge", "GET", 405u16)] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let b = bridge.clone();
            tokio::spawn(async move {
                if let Ok((mut socket, _)) = listener.accept().await {
                    let _ = handle_connection(&b, &mut socket).await;
                }
            });
            let client = reqwest::Client::new();
            let req = if method == "GET" {
                client
                    .get(format!("http://127.0.0.1:{port}{path}"))
                    .header("X-NeoBrowser-Token", bridge.token())
            } else {
                client
                    .post(format!("http://127.0.0.1:{port}{path}"))
                    .header("X-NeoBrowser-Token", bridge.token())
                    .json(&json!({}))
            };
            let res = req.send().await.unwrap();
            assert_eq!(res.status().as_u16(), expect, "{method} {path}");
        }
    }
}
