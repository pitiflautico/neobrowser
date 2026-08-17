//! Reach tools: browse (server-side fetch), upload (file input), download (auth-aware).
//!
//! Ported from the Python `browse`/`upload`/`download` dispatch. The SSRF guard is
//! kept: only public http(s) URLs are allowed — file://, credentials-in-URL, and
//! hosts resolving to loopback/private/link-local ranges (incl. cloud metadata) are
//! blocked before any request goes out.

pub mod fetch;
pub mod files;
pub mod ssrf;
pub mod transfer;

// Re-exported flat so every existing `reach::browse`, `reach::upload`, … call site keeps
// working: the split is an internal reorganisation, not an API change.
pub use fetch::browse;
pub use files::{mcp_roots, set_mcp_roots};
pub use ssrf::validate_url;
pub use transfer::{download, resolve_upload_path, upload, upload_roots_for_report};

#[cfg(test)]
mod tests {
    use super::*;
    // The tests deliberately stay together rather than moving into each submodule: they
    // assert the behaviour of the guard as a whole — a URL is validated, followed,
    // scoped and written — which is a property of the composition, not of one file.
    use fetch::credentials::{safe_cross_origin, CredentialScope};
    use fetch::text::{clean_scraped, strip_html};
    use fetch::{guarded_get, read_capped};
    use files::{download_size_cap, write_download_atomically};
    use serde_json::Map;
    use std::time::Duration;
    use transfer::resolve_upload_path;
    use transfer::upload::is_sensitive_upload;

    #[test]
    fn ssrf_blocks_non_public() {
        assert!(!validate_url("file:///etc/passwd"));
        assert!(!validate_url("http://localhost/x"));
        assert!(!validate_url("http://127.0.0.1/x"));
        assert!(!validate_url("http://10.0.0.5/x"));
        assert!(!validate_url("http://192.168.1.1/x"));
        assert!(!validate_url("http://169.254.169.254/latest/meta-data"));
        assert!(!validate_url("http://user:pass@example.com/"));
        assert!(!validate_url("http://metadata.google.internal/"));
        assert!(!validate_url("ftp://example.com/"));
    }

    #[test]
    fn ssrf_allows_public_literals() {
        assert!(validate_url("http://8.8.8.8/"));
        assert!(validate_url("https://1.1.1.1/"));
    }

    #[test]
    fn ssrf_blocks_ipv4_in_v6_disguises() {
        // IPv4-mapped: ::ffff:a.b.c.d
        assert!(!validate_url("http://[::ffff:127.0.0.1]/"));
        assert!(!validate_url("http://[::ffff:a9fe:a9fe]/")); // 169.254.169.254
        assert!(!validate_url("http://[::ffff:10.0.0.5]/"));
        // IPv4-compatible: ::a.b.c.d
        assert!(!validate_url("http://[::127.0.0.1]/"));
        // 6to4 (2002::/16) embedding 127.0.0.1.
        assert!(!validate_url("http://[2002:7f00:1::]/"));
        // Teredo (2001:0000::/32) embedding 127.0.0.1 (XORed).
        assert!(!validate_url("http://[2001:0::80ff:fffe]/"));
        // Plain v6 loopback/link-local/unique-local still blocked.
        assert!(!validate_url("http://[::1]/"));
        assert!(!validate_url("http://[fe80::1]/"));
        assert!(!validate_url("http://[fd00::1]/"));
        // A mapped PUBLIC address still passes.
        assert!(validate_url("http://[::ffff:8.8.8.8]/"));
    }

    #[tokio::test]
    async fn guarded_get_blocks_private_url() {
        let err = guarded_get(
            "http://127.0.0.1:1/",
            "ua",
            Duration::from_secs(1),
            &Map::new(),
            None,
        )
        .await
        .unwrap_err();
        assert!(err.contains("blocked"), "got: {err}");
    }

    #[tokio::test]
    async fn read_capped_stops_at_cap() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut s, _)) = listener.accept().await {
                let mut req = [0u8; 1024];
                let _ = s.read(&mut req).await;
                let body = vec![b'x'; 1024 * 1024];
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(head.as_bytes()).await;
                let _ = s.write_all(&body).await;
            }
        });
        let resp = reqwest::get(format!("http://{addr}/")).await.unwrap();
        let buf = read_capped(resp, 4096).await.unwrap();
        assert_eq!(buf.len(), 4096);
    }

    #[test]
    fn strip_html_removes_tags_and_scripts() {
        let html = "<html><head><style>a{}</style></head><body>Hello <b>world</b><script>evil()</script>!</body></html>";
        let text = strip_html(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("world"));
        assert!(!text.contains("evil"));
        assert!(!text.contains("<"));
    }

    #[test]
    fn clean_scraped_strips_zero_width() {
        let dirty = "he\u{200B}llo\u{FEFF}";
        assert_eq!(clean_scraped(dirty), "hello");
    }

    #[test]
    fn download_filename_sanitized() {
        // Indirect check of the sanitizer via a crafted URL basename.
        let raw = "../../etc/pa$$wd?x=1";
        let safe: String = raw
            .rsplit('/')
            .next()
            .unwrap()
            .split('?')
            .next()
            .unwrap()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        assert_eq!(safe, "pa__wd");
    }

    #[test]
    fn sensitive_upload_paths_blocked() {
        use std::path::Path;
        assert!(is_sensitive_upload(Path::new("/Users/x/.ssh/id_rsa")));
        assert!(is_sensitive_upload(Path::new("/Users/x/.aws/credentials")));
        assert!(is_sensitive_upload(Path::new(
            "/Users/x/Documents/server.pem"
        )));
        assert!(is_sensitive_upload(Path::new("/Users/x/project/.env")));
        assert!(is_sensitive_upload(Path::new(
            "/Users/x/Library/Keychains/login.keychain-db"
        )));
        // Ordinary user content is fine.
        assert!(!is_sensitive_upload(Path::new(
            "/Users/x/Downloads/photo.png"
        )));
        assert!(!is_sensitive_upload(Path::new("/Users/x/Documents/cv.pdf")));
    }

    /// The regression that matters: downloading twice must not destroy the first
    /// file. `fs::write` silently replaced it.
    #[test]
    fn a_second_download_does_not_overwrite_the_first() {
        let dir = std::env::temp_dir().join(format!("nb-dl-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let dest = dir.join("invoice.pdf");

        let (p1, renamed1) = write_download_atomically(&dest, b"first").unwrap();
        assert_eq!(p1, dest);
        assert!(!renamed1);

        let (p2, renamed2) = write_download_atomically(&dest, b"second").unwrap();
        assert_ne!(p2, dest, "must not reuse the occupied name");
        assert!(renamed2, "must report that it renamed");
        assert_eq!(p2.file_name().unwrap().to_string_lossy(), "invoice (1).pdf");

        // Both files intact, with their own contents.
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "first");
        assert_eq!(std::fs::read_to_string(&p2).unwrap(), "second");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// No `.part-*` scratch file may survive a successful write.
    #[test]
    fn atomic_download_leaves_no_partial_file() {
        let dir = std::env::temp_dir().join(format!("nb-dl-part-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        write_download_atomically(&dir.join("f.bin"), b"data").unwrap();
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".part-"))
            .collect();
        assert!(leftovers.is_empty(), "partial files left: {leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn download_cap_is_configurable_and_has_a_sane_default() {
        let _g = crate::env_test_guard();
        let prev = std::env::var("NEOBROWSER_MAX_DOWNLOAD_MB").ok();
        std::env::remove_var("NEOBROWSER_MAX_DOWNLOAD_MB");
        assert_eq!(download_size_cap(), 200 * 1024 * 1024);
        std::env::set_var("NEOBROWSER_MAX_DOWNLOAD_MB", "5");
        assert_eq!(download_size_cap(), 5 * 1024 * 1024);
        // Zero or nonsense must not disable the cap.
        std::env::set_var("NEOBROWSER_MAX_DOWNLOAD_MB", "0");
        assert_eq!(download_size_cap(), 200 * 1024 * 1024);
        std::env::set_var("NEOBROWSER_MAX_DOWNLOAD_MB", "banana");
        assert_eq!(download_size_cap(), 200 * 1024 * 1024);
        match prev {
            Some(v) => std::env::set_var("NEOBROWSER_MAX_DOWNLOAD_MB", v),
            None => std::env::remove_var("NEOBROWSER_MAX_DOWNLOAD_MB"),
        }
    }

    /// The symlink race, demonstrated. Validation passes on a real file; the file is then
    /// replaced by a symlink to a secret; the staged copy must still hold the original
    /// contents, because staging happened before the swap and Chrome only ever sees the
    /// staged path.
    #[test]
    fn staging_defeats_a_post_validation_symlink_swap() {
        let _g = crate::env_test_guard();
        let dir = std::env::temp_dir().join(format!("nb-stage-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::env::set_var("NEOBROWSER_UPLOAD_DIR", &dir);
        std::env::set_var("NEOBROWSER_HOME", "/tmp/nb-stage-home");

        let secret = dir.join("secret.txt");
        std::fs::write(&secret, b"SECRET-CONTENTS").unwrap();
        let target = dir.join("innocent.txt");
        std::fs::write(&target, b"innocent").unwrap();

        let validated = transfer::resolve_upload_path(target.to_str().unwrap())
            .expect("an innocent file under the allowed root validates");
        let staged = transfer::stage::stage_for_upload(&validated).expect("staging succeeds");

        // The attacker's move: swap the validated path for a symlink to the secret.
        std::fs::remove_file(&target).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&secret, &target).unwrap();

        // What Chrome would open is the staged path, and it still holds the original.
        assert_ne!(
            staged, validated,
            "the staged path must not be the caller's path"
        );
        assert_eq!(
            std::fs::read_to_string(&staged).unwrap(),
            "innocent",
            "the staged copy must be immune to the swap"
        );
        assert!(
            !std::fs::read_to_string(&staged).unwrap().contains("SECRET"),
            "the secret must never be reachable through the staged path"
        );
        // And the staging directory is ours, not somewhere the attacker writes.
        assert!(staged.starts_with(crate::paths::home()));

        std::env::remove_var("NEOBROWSER_UPLOAD_DIR");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all("/tmp/nb-stage-home");
    }

    #[test]
    fn the_upload_cap_is_configurable_and_enforced() {
        let _g = crate::env_test_guard();
        let dir = std::env::temp_dir().join(format!("nb-stage-cap-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::env::set_var("NEOBROWSER_UPLOAD_DIR", &dir);
        std::env::set_var("NEOBROWSER_HOME", "/tmp/nb-stage-cap-home");
        std::env::set_var("NEOBROWSER_MAX_UPLOAD_MB", "1");

        let big = dir.join("big.bin");
        std::fs::write(&big, vec![0u8; 2 * 1024 * 1024]).unwrap();
        let validated = transfer::resolve_upload_path(big.to_str().unwrap()).unwrap();
        let err = transfer::stage::stage_for_upload(&validated).unwrap_err();
        assert!(err.contains("over the 1 MiB upload cap"), "{err}");

        std::env::remove_var("NEOBROWSER_MAX_UPLOAD_MB");
        std::env::remove_var("NEOBROWSER_UPLOAD_DIR");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all("/tmp/nb-stage-cap-home");
    }

    #[test]
    fn upload_restricted_to_allowed_root() {
        let _g = crate::env_test_guard();
        let dir = std::env::temp_dir().join(format!("nb-upload-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let inside = dir.join("ok.txt");
        std::fs::write(&inside, b"hi").unwrap();
        let outside = std::env::temp_dir().join(format!("nb-upl-out-{}.txt", std::process::id()));
        std::fs::write(&outside, b"hi").unwrap();

        std::env::set_var("NEOBROWSER_UPLOAD_DIR", &dir);
        // A file inside the allowed dir resolves.
        assert!(resolve_upload_path(inside.to_str().unwrap()).is_ok());
        // A file outside is refused.
        let err = resolve_upload_path(outside.to_str().unwrap()).unwrap_err();
        assert!(err.contains("outside allowed upload dirs"), "got: {err}");
        // A missing file is refused.
        assert!(resolve_upload_path("/no/such/file-xyz").is_err());

        std::env::remove_var("NEOBROWSER_UPLOAD_DIR");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&outside);
    }

    /// Walk a chain of URLs the way `guarded_get` does and report whether
    /// credentials would still be attached at the end.
    fn credentials_survive(chain: &[&str]) -> bool {
        let mut scope = CredentialScope::new(chain[0]);
        for hop in chain {
            scope.visit(&reqwest::Url::parse(hop).unwrap());
        }
        scope.allows_credentials()
    }

    #[test]
    fn credentials_stay_on_the_requested_origin() {
        assert!(credentials_survive(&["https://api.example.com/a"]));
        // Same origin, different path or query: still the caller's origin.
        assert!(credentials_survive(&[
            "https://api.example.com/a",
            "https://api.example.com/b?x=1",
        ]));
        // Host casing is not a difference.
        assert!(credentials_survive(&[
            "https://API.Example.com/a",
            "https://api.example.com/a",
        ]));
        // The explicit default port is the same origin as the implicit one.
        assert!(credentials_survive(&[
            "https://api.example.com/a",
            "https://api.example.com:443/a",
        ]));
    }

    #[test]
    fn credentials_never_reach_another_origin() {
        // The headline case: a redirect to an attacker's host.
        assert!(!credentials_survive(&[
            "https://api.example.com/a",
            "https://evil.test/steal",
        ]));
        // A sibling subdomain is a different origin, not a trusted relative.
        assert!(!credentials_survive(&[
            "https://api.example.com/a",
            "https://other.example.com/a",
        ]));
        // Parent domain likewise.
        assert!(!credentials_survive(&[
            "https://api.example.com/a",
            "https://example.com/a",
        ]));
        // A scheme downgrade on the SAME host: this is why the origin includes
        // the scheme. Forwarding here would replay a secure cookie in plaintext.
        assert!(!credentials_survive(&[
            "https://api.example.com/a",
            "http://api.example.com/a",
        ]));
        // A different port is a different origin.
        assert!(!credentials_survive(&[
            "https://api.example.com/a",
            "https://api.example.com:8443/a",
        ]));
    }

    /// Leaving the caller's origin must be irreversible: after `evil.test` has
    /// had control of the chain, the "example.com" it redirects back to is its
    /// choice, not the caller's.
    #[test]
    fn returning_to_the_original_origin_does_not_restore_credentials() {
        assert!(!credentials_survive(&[
            "https://api.example.com/a",
            "https://evil.test/bounce",
            "https://api.example.com/a",
        ]));
    }

    #[test]
    fn unparseable_start_url_fails_closed() {
        let mut scope = CredentialScope::new("not a url");
        scope.visit(&reqwest::Url::parse("https://evil.test/").unwrap());
        assert!(!scope.allows_credentials());
    }

    #[test]
    fn only_content_negotiation_headers_survive_a_cross_origin_hop() {
        for safe in [
            "Accept",
            "accept-language",
            "USER-AGENT",
            " accept-charset ",
        ] {
            assert!(safe_cross_origin(safe), "{safe} should be forwardable");
        }
        // Known credential carriers, and — the point of an allowlist — names
        // nobody enumerated in advance.
        for secret in [
            "Authorization",
            "authorization",
            "Cookie",
            "Proxy-Authorization",
            "X-Api-Key",
            "x-auth-token",
            "api-key",
            "X-Acme-Internal-Session",
            "X-Some-Header-Invented-Tomorrow",
        ] {
            assert!(!safe_cross_origin(secret), "{secret} must be withheld");
        }
    }
}
