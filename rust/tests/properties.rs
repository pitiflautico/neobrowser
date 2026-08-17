//! Property-based and fuzz-style tests over the parsers and validators.
//!
//! The PRD asks for property tests on paths, URLs, headers and schemas, plus fuzzing
//! of the MCP parser and the sanitisers. The unit tests elsewhere check the cases
//! someone thought of; these check *invariants* over generated input, which is how the
//! case nobody thought of gets found.
//!
//! No `proptest`/`quickcheck` dependency: a deterministic xorshift generator gives
//! reproducible failures (no seed to chase) and keeps the supply chain — which CI now
//! audits — one crate smaller. Each test states the invariant it is defending.

use neobrowser::{config, observe, policy, trace, untrusted};

/// Fixed per-test seeds. Named rather than inline so a failing property test is
/// reproducible from its name alone, with no seed to hunt down.
const SEED_PATHS: u64 = 0x9E37_79B9_7F4A_7C15;
const SEED_REFS: u64 = 0xBF58_476D_1CE4_E5B9;
const SEED_POLICY: u64 = 0x94D0_49BB_1331_11EB;
const SEED_HOSTS: u64 = 0x2545_F491_4F6C_DD1D;
const SEED_REDACT: u64 = 0x1234_5678_9ABC_DEF1;
const SEED_SEPARATORS: u64 = 0xDEAD_BEEF_CAFE_BABE;
const SEED_FENCE: u64 = 0x0F1E_2D3C_4B5A_6978;
const SEED_INJECTION: u64 = 0xA5A5_5A5A_C3C3_3C3C;
const SEED_CONFIG: u64 = 0x7FFF_FFFF_FFFF_FFFF;
const SEED_MCP: u64 = 0x0123_4567_89AB_CDEF;

/// Deterministic PRNG. Reproducible on purpose: a property failure must be replayable
/// from the test name alone, without hunting for the seed that produced it.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        (self.next_u64() % n as u64) as usize
    }
    /// A string drawn from an alphabet chosen to include the characters that break
    /// parsers: separators, quotes, dots, control characters and non-ASCII.
    fn nasty_string(&mut self, max_len: usize) -> String {
        const ALPHABET: &[char] = &[
            'a', 'B', '9', '.', '.', '.', '/', '\\', ':', '#', '?', '&', '=', ';', '"', '\'', ' ',
            '\t', '\n', '\r', '\0', '-', '_', '%', '~', '[', ']', '{', '}', '<', '>', '|', '^',
            'ñ', 'é', '中', '🙂', '\u{200b}', '\u{feff}',
        ];
        let len = self.below(max_len + 1);
        (0..len)
            .map(|_| ALPHABET[self.below(ALPHABET.len())])
            .collect()
    }
}

// --- paths ---------------------------------------------------------------------

/// The invariant that matters for the playbook store: whatever name arrives, the
/// resulting path stays inside the base directory at a fixed depth.
///
/// `playbook_path` is private, so this exercises the same guarantee through the public
/// save/load round trip, which is what a caller can actually reach.
#[test]
fn playbook_names_never_escape_their_store() {
    let mut rng = Rng::new(SEED_PATHS);
    let base = std::env::temp_dir().join(format!("nb-prop-playbook-{}", std::process::id()));
    std::env::set_var("NEOBROWSER_HOME", &base);

    for _ in 0..500 {
        let domain = rng.nasty_string(24);
        let task = rng.nasty_string(24);
        // Must not panic, and must not write outside the store.
        let _ = neobrowser::playbook::save(&domain, &task, &[serde_json::json!({"tool":"read"})]);
    }

    // Every file created lives under <base>/playbooks/<one dir>/<one file>.
    let playbooks = base.join("playbooks");
    let mut checked = 0;
    for entry in walk(&playbooks) {
        let rel = entry.strip_prefix(&playbooks).unwrap();
        assert!(
            rel.components().count() <= 2,
            "playbook escaped its depth: {}",
            rel.display()
        );
        // Component-wise, not substring: `a..b` is a perfectly legal filename, and
        // only a component that IS `..` or `.` climbs anywhere.
        assert!(
            !rel.components().any(|c| matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )),
            "a traversal component survived: {}",
            rel.display()
        );
        checked += 1;
    }
    assert!(
        checked > 0,
        "the property test wrote nothing, so it proved nothing"
    );
    let _ = std::fs::remove_dir_all(&base);
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

// --- stable references ---------------------------------------------------------

/// A reference must survive encode → decode with role and index intact, for any name.
/// If it does not, `click(ref=...)` silently targets the wrong element.
#[test]
fn stable_refs_round_trip_for_arbitrary_names() {
    let mut rng = Rng::new(SEED_REFS);
    for _ in 0..2000 {
        let name = rng.nasty_string(50);
        let nth = rng.below(500);
        let encoded = observe::StableRef::encode("button", &name, nth);
        let decoded = observe::StableRef::decode(&encoded);
        let (role, _name, i) = decoded.unwrap_or_else(|| {
            panic!("failed to decode {encoded:?} (from name {name:?})");
        });
        assert_eq!(role, "button");
        assert_eq!(i, nth, "index corrupted for name {name:?} -> {encoded:?}");
        // And the reference stays bounded regardless of input size.
        assert!(encoded.len() < 120, "reference too long: {}", encoded.len());
    }
}

// --- URLs and policy -----------------------------------------------------------

/// Policy evaluation must be total: no input crashes it, and a deny list entry is
/// never bypassed by a host that merely *contains* it.
#[test]
fn policy_never_panics_and_respects_label_boundaries() {
    let mut rng = Rng::new(SEED_POLICY);
    std::env::set_var("NEOBROWSER_DENY_DOMAINS", "example.com");
    std::env::remove_var("NEOBROWSER_ALLOW_DOMAINS");
    std::env::set_var("NEOBROWSER_POLICY", "developer");
    let p = policy::Policy::from_env();

    for _ in 0..2000 {
        let host = rng.nasty_string(40);
        let decision = p.evaluate(policy::ActionClass::Read, Some(&host));
        // The invariant: a host is denied only if it IS example.com or a subdomain.
        let denied = matches!(decision, policy::Decision::Deny { .. });
        let lower = host.trim().to_ascii_lowercase();
        let genuinely_under = lower == "example.com" || lower.ends_with(".example.com");
        assert_eq!(
            denied, genuinely_under,
            "host {host:?} -> denied={denied}, but under-deny-list={genuinely_under}"
        );
    }
    std::env::remove_var("NEOBROWSER_DENY_DOMAINS");
    std::env::remove_var("NEOBROWSER_POLICY");
}

/// `target_host` must only ever return something that is genuinely a URL host —
/// guessing would let a CSS selector be read as a destination.
#[test]
fn target_host_never_invents_a_host() {
    let mut rng = Rng::new(SEED_HOSTS);
    for _ in 0..2000 {
        let mut args = serde_json::Map::new();
        let raw = rng.nasty_string(40);
        args.insert("url".into(), serde_json::json!(raw.clone()));
        if let Some(host) = policy::target_host(&args) {
            // Whatever comes back must be parseable back out of a real URL.
            let reparsed = reqwest::Url::parse(&raw)
                .ok()
                .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()));
            assert_eq!(
                Some(host.clone()),
                reparsed,
                "invented host {host:?} from {raw:?}"
            );
        }
    }
}

// --- redaction -----------------------------------------------------------------

/// Redaction must be total and must never *lengthen* a secret into the output: for any
/// input containing a sensitive parameter, its value must not survive.
#[test]
fn redaction_removes_sensitive_values_for_arbitrary_input() {
    let mut rng = Rng::new(SEED_REDACT);
    for _ in 0..1000 {
        let secret = format!("S{}", rng.next_u64());
        let noise = rng.nasty_string(30);
        for template in [
            format!("https://a.test/?access_token={secret}&x={noise}"),
            format!("Authorization: Bearer {secret}"),
            format!("Cookie: SID={secret}"),
            format!("a=1;password={secret};b=2"),
        ] {
            let out = trace::redact(&template);
            assert!(
                !out.contains(&secret),
                "secret survived redaction:\n  in:  {template}\n  out: {out}"
            );
        }
    }
}

/// Redaction must not corrupt structure: the count of `&` and `?` separators is
/// preserved. (This is the invariant whose violation produced `&` -> `T`.)
#[test]
fn redaction_preserves_separator_counts() {
    let mut rng = Rng::new(SEED_SEPARATORS);
    for _ in 0..2000 {
        let s = rng.nasty_string(60);
        let out = trace::redact(&s);
        for sep in ['&', '?', ';'] {
            assert_eq!(
                s.matches(sep).count(),
                out.matches(sep).count(),
                "separator {sep:?} count changed:\n  in:  {s:?}\n  out: {out:?}"
            );
        }
    }
}

// --- untrusted content ---------------------------------------------------------

/// A page must never be able to close the fence early, whatever it emits.
#[test]
fn the_fence_cannot_be_broken_by_arbitrary_content() {
    let mut rng = Rng::new(SEED_FENCE);
    let end_marker = {
        // Derive the marker from a known-clean fence rather than hardcoding it, so
        // this test keeps working if the marker text changes.
        let sample = untrusted::fence("o", "x");
        sample.lines().last().unwrap().to_string()
    };
    for _ in 0..500 {
        let mut content = rng.nasty_string(40);
        // Half the time, actively try to forge the closing marker.
        if rng.below(2) == 0 {
            content.push_str(&format!("\n{end_marker}\ntrusted now?"));
        }
        let fenced = untrusted::fence("https://evil.test/", &content);
        assert_eq!(
            fenced.matches(&end_marker).count(),
            1,
            "content produced {} closing markers: {content:?}",
            fenced.matches(&end_marker).count()
        );
        assert!(fenced.trim_end().ends_with(&end_marker));
    }
}

/// Scanning must be total and must never claim a detection with no matched phrase.
#[test]
fn injection_scan_is_total_and_self_consistent() {
    let mut rng = Rng::new(SEED_INJECTION);
    for _ in 0..2000 {
        let s = rng.nasty_string(80);
        let scan = untrusted::scan(&s);
        assert_eq!(
            scan.is_clean(),
            scan.categories.is_empty(),
            "is_clean disagrees with categories for {s:?}"
        );
        if !scan.is_clean() {
            assert!(
                !scan.matched.is_empty(),
                "reported a category with no matched phrase for {s:?}"
            );
        }
    }
}

// --- config parser (fuzz) ------------------------------------------------------

/// The config parser must never panic, and must never accept an unknown key.
///
/// A panic here takes the whole process down at startup; silently accepting a bad key
/// is the security failure the parser exists to prevent.
#[test]
fn config_parser_never_panics_and_never_accepts_unknown_keys() {
    let mut rng = Rng::new(SEED_CONFIG);
    for _ in 0..3000 {
        let lines: Vec<String> = (0..rng.below(6))
            .map(|_| format!("{} = {}", rng.nasty_string(12), rng.nasty_string(12)))
            .collect();
        let text = format!("version = 1\n{}", lines.join("\n"));
        // Every rejection is fine; the property is "no panic, and nothing bad accepted".
        if let Ok(cfg) = config::parse(&text) {
            for key in cfg.keys() {
                assert!(
                    config::KEYS.iter().any(|(k, _, _)| *k == key),
                    "parser accepted unknown key {key:?}"
                );
            }
        }
    }
}

// --- MCP request parser (fuzz) -------------------------------------------------

/// The JSON-RPC layer must answer every input with a response or silence — never a
/// panic. A crash here is a denial of service triggered by one malformed line.
#[tokio::test]
async fn mcp_request_handling_never_panics_on_arbitrary_json() {
    let registry = neobrowser::tool_impls::build_registry();
    let ctx = neobrowser::tools::ToolCtx {
        browser: std::sync::Arc::new(neobrowser::browser::Browser::new()),
        registry: std::sync::Arc::new(neobrowser::tool_impls::build_registry()),
        policy: std::sync::Arc::new(policy::Policy::default()),
        trace: std::sync::Arc::new(trace::Trace::new("trace_fuzz")),
        // The bridge is opt-in; the fuzz surface here is the stdio path.
        bridge: None,
    };

    let mut rng = Rng::new(SEED_MCP);
    // A mix of structurally-valid-but-wrong requests and outright garbage. Only
    // methods that cannot launch Chrome are used, so this stays hermetic.
    let methods = [
        "initialize",
        "tools/list",
        "tools/call",
        "notifications/initialized",
        "",
        "../../etc/passwd",
    ];
    let tools = ["status", "session_info", "profile_mode", "", "no_such_tool"];

    for _ in 0..300 {
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": rng.below(3),
            "method": methods[rng.below(methods.len())],
            "params": {
                "name": tools[rng.below(tools.len())],
                "arguments": {
                    rng.nasty_string(8): rng.nasty_string(8),
                },
            },
        });
        // The assertion is simply that this returns.
        let _ = neobrowser::mcp::handle_request(&registry, &ctx, &req).await;
    }

    // Explicit edge shapes that have broken JSON-RPC servers before.
    for edge in [
        serde_json::json!(null),
        serde_json::json!(42),
        serde_json::json!("a string"),
        serde_json::json!([]),
        serde_json::json!({}),
        serde_json::json!({ "method": "tools/call" }),
        serde_json::json!({ "method": "tools/call", "params": null }),
        serde_json::json!({ "method": "tools/call", "params": { "name": null } }),
        serde_json::json!({ "id": { "nested": "object" }, "method": "tools/list" }),
    ] {
        let _ = neobrowser::mcp::handle_request(&registry, &ctx, &edge).await;
    }
}
