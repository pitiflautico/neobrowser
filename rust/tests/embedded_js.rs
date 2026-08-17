//! Syntax-check every JavaScript snippet embedded in the Rust source.
//!
//! This exists because of a real bug. `action::state_js` was built with
//! `format!("return {STATE_JS}")` where the snippet began with a newline, so the
//! generated code was `return\n(function…)` — JavaScript's automatic semicolon
//! insertion turned that into a bare `return;`. Every observation silently returned
//! `undefined`, so every verified action reported `uncertain`. Nothing caught it: the
//! Rust compiled, the tests passed, and the failure only appeared when driving a real
//! page.
//!
//! Embedded JS has no compiler. This file is the compiler, in three layers:
//!
//! 1. **Nothing is embedded any more.** Every snippet now lives in `js/`, so
//!    `no_javascript_remains_inline_in_rust` asserts the inline count is zero and the
//!    `node --check` pass over inline snippets is a backstop with nothing to do.
//! 2. **Each `js/*.js` file parses on its own** — the check an editor or linter would do.
//! 3. **Each snippet parses in the form `page::js` actually builds for it**, which is not
//!    one shape: an expression arrives as `return <expr>` inside an async IIFE, a statement
//!    sequence arrives as written. That layer is the one whose absence produced the bug
//!    above, and it is also where the ASI hazard is checked structurally, since `return;`
//!    followed by an expression parses fine and merely does the wrong thing.
//!
//! Self-skips when Node is absent, matching how the live-Chrome tests self-skip without
//! Chrome: a contributor without Node should not see a spurious failure. CI has Node.

use std::io::Write;
use std::path::Path;

/// A snippet found in the source, with enough context to report it usefully.
struct Snippet {
    file: String,
    line: usize,
    code: String,
}

/// Does this raw string look like JavaScript rather than SQL, JSON, or prose?
fn looks_like_js(body: &str) -> bool {
    let markers = [
        "function",
        "document.",
        "querySelector",
        "=>",
        "return ",
        "window.",
        "navigator.",
        "performance.",
    ];
    markers.iter().filter(|m| body.contains(*m)).count() >= 1
}

/// Extract `r#"…"#` raw strings that look like JS.
///
/// Only raw strings: an ordinary `"…"` literal containing JS is short enough to read at
/// a glance, and parsing escaped quotes correctly is not worth the complexity here.
fn extract_snippets(src: &str, file: &str) -> Vec<Snippet> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    while i + 3 < bytes.len() {
        if &bytes[i..i + 3] == b"r#\"" {
            let start = i + 3;
            // Find the matching `"#`.
            let mut j = start;
            while j + 1 < bytes.len() && !(bytes[j] == b'"' && bytes[j + 1] == b'#') {
                j += 1;
            }
            if j + 1 >= bytes.len() {
                break;
            }
            let body = &src[start..j];
            if looks_like_js(body) {
                out.push(Snippet {
                    file: file.to_string(),
                    line: src[..start].matches('\n').count() + 1,
                    code: body.to_string(),
                });
            }
            i = j + 2;
        } else {
            i += 1;
        }
    }
    out
}

/// Turn a snippet into something Node can parse.
///
/// Two transformations, both mirroring what `format!` does at runtime:
/// - `{{` / `}}` become `{` / `}` — the escaping that makes a JS object literal survive
///   `format!`, and the most common source of a brace mismatch.
/// - `{ident}` placeholders become `null`. The substituted values are strings, numbers
///   and JSON at runtime, and `null` is syntactically valid in every position they
///   occupy, so this checks the snippet's structure without needing the real values.
fn prepare_for_node(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let chars: Vec<char> = code.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '{' if i + 1 < chars.len() && chars[i + 1] == '{' => {
                out.push('{');
                i += 2;
            }
            '}' if i + 1 < chars.len() && chars[i + 1] == '}' => {
                out.push('}');
                i += 2;
            }
            '{' => {
                // A `{ident}` placeholder: consume to the closing brace.
                let mut j = i + 1;
                let mut ident = String::new();
                while j < chars.len() && chars[j] != '}' {
                    ident.push(chars[j]);
                    j += 1;
                }
                let is_placeholder = !ident.is_empty()
                    && ident.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                if is_placeholder && j < chars.len() {
                    out.push_str("null");
                    i = j + 1;
                } else {
                    out.push('{');
                    i += 1;
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// Wrap a snippet so it parses standalone.
///
/// Many snippets are function *bodies* passed to `page::js`, which wraps them in an
/// async IIFE at runtime. Applying the same wrapper here means a bare `return …`
/// snippet is checked as the code it actually becomes — including whether the `return`
/// and its expression end up on the same line, which is the ASI bug.
fn wrap_like_runtime(code: &str) -> String {
    if code.contains("return ") {
        format!("(async function(){{{code}}})()")
    } else {
        code.to_string()
    }
}

/// Every `.rs` file under `src/`, recursively.
///
/// Recursive on purpose: `tool_impls` became a directory when it was split, and a
/// checker that only globbed `src/*.rs` would have quietly stopped covering anything
/// moved into a subdirectory — a protection that silently narrows is worse than none,
/// because it still reports success.
fn all_rust_sources(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(all_rust_sources(&path));
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    out
}

/// Collect every JS snippet in the crate.
fn all_snippets() -> Vec<Snippet> {
    let mut snippets = Vec::new();
    for path in all_rust_sources(Path::new("src")) {
        let src = std::fs::read_to_string(&path).expect("readable source file");
        // The path relative to src/, so a report points at the right file after the split.
        let name = path
            .strip_prefix("src")
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        snippets.extend(extract_snippets(&src, &name));
    }
    snippets
}

fn node_available() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// No JavaScript is left inline in the Rust source.
///
/// This used to be a floor — "at least 12 snippets, and every one of them parses" — because
/// inline JS was the normal case. It is now zero: every snippet lives in `js/`, where it is
/// syntax-checked as a real file and its wrapped form is checked against `page::js`. So the
/// assertion is inverted. Inline JS is not a syntax risk so much as an *unchecked* one: a
/// snippet in a `format!` string is invisible to `every_extracted_js_file_parses` and to
/// `every_extracted_snippet_parses_in_the_form_that_reaches_the_browser`, which is the check
/// whose absence produced the ASI bug in the first place.
///
/// `every_embedded_js_snippet_parses` below still `node --check`s anything that does appear,
/// so a snippet added in a hurry is not completely unguarded — but this is where it fails.
#[test]
fn no_javascript_remains_inline_in_rust() {
    let offenders: Vec<String> = all_snippets()
        .iter()
        .map(|s| format!("{}:{}", s.file, s.line))
        .collect();
    assert!(
        offenders.is_empty(),
        "JavaScript is embedded in Rust at {} site(s):\n  {}\nPut it in js/ as a real file, \
         load it with `include_str!` through `crate::js`, and register it in \
         `js::all_snippets_for_test` so its wrapped form is checked too",
        offenders.len(),
        offenders.join("\n  ")
    );
}

/// The extractor must actually find JS, or the test above passes while verifying nothing.
///
/// This is what replaces the old `>= 12` floor. That floor guarded against a broken
/// extractor by counting real snippets, which stops working the moment there are none — so
/// the extractor is exercised against a fixture instead, which keeps working at zero.
#[test]
fn the_extractor_finds_embedded_js() {
    let src = r###"
        let a = format!(r#"return document.querySelector({sel}).value"#, sel = x);
        let b = r#"SELECT * FROM cookies WHERE host_key = ?"#;
        let c = r#"{"ok": false}"#;
    "###;
    let found = extract_snippets(src, "fixture.rs");
    assert_eq!(
        found.len(),
        1,
        "expected exactly the JS raw string, got {:?}",
        found.iter().map(|s| &s.code).collect::<Vec<_>>()
    );
    assert!(found[0].code.contains("querySelector"));
    // SQL and JSON must not be mistaken for JS, or the guard above would fire on every
    // query in the crate and get weakened to make it pass.
    assert!(!looks_like_js("SELECT * FROM cookies WHERE host_key = ?"));
    assert!(!looks_like_js(r#"{"ok": false}"#));
}

/// A backstop, not the main protection: `no_javascript_remains_inline_in_rust` forbids the
/// input this test consumes, so it normally has nothing to do. It stays because if someone
/// does add an inline snippet, a syntax error in it should surface here rather than in a
/// browser.
#[test]
fn every_embedded_js_snippet_parses() {
    if !node_available() {
        eprintln!("skipping: node is not installed (CI has it)");
        return;
    }

    let snippets = all_snippets();
    let tmp = std::env::temp_dir().join(format!("nb-jscheck-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("temp dir");

    let mut failures = Vec::new();
    for (n, s) in snippets.iter().enumerate() {
        let prepared = wrap_like_runtime(&prepare_for_node(&s.code));
        let file = tmp.join(format!("snippet-{n}.js"));
        {
            let mut f = std::fs::File::create(&file).expect("write snippet");
            f.write_all(prepared.as_bytes()).expect("write snippet");
        }
        let out = std::process::Command::new("node")
            .arg("--check")
            .arg(&file)
            .output()
            .expect("run node --check");
        if !out.status.success() {
            failures.push(format!(
                "{}:{} — {}",
                s.file,
                s.line,
                String::from_utf8_lossy(&out.stderr)
                    .lines()
                    .find(|l| l.contains("SyntaxError"))
                    .unwrap_or("syntax error")
                    .trim()
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);

    assert!(
        failures.is_empty(),
        "embedded JavaScript failed to parse:\n  {}",
        failures.join("\n  ")
    );
    eprintln!("checked {} embedded JS snippets", snippets.len());
}

/// The `format!` escaping is the other silent hazard: an unbalanced `{{`/`}}` produces
/// JS with mismatched braces, which `prepare_for_node` must reproduce faithfully or the
/// check above would pass on broken code.
#[test]
fn the_preparer_reproduces_format_escaping() {
    assert_eq!(prepare_for_node("{{ a: 1 }}"), "{ a: 1 }");
    assert_eq!(prepare_for_node("f({sel})"), "f(null)");
    assert_eq!(prepare_for_node("fnv(s, {salt})"), "fnv(s, null)");
    // A brace that is neither escaped nor a placeholder is left alone.
    assert_eq!(prepare_for_node("if (x) { y() }"), "if (x) { y() }");
    // Nested escaping, as it appears in real snippets.
    assert_eq!(
        prepare_for_node("JSON.stringify({{ok: true}})"),
        "JSON.stringify({ok: true})"
    );
}

/// The wrapper must put `return` and its expression on the same line, because that is
/// precisely the bug this file exists to prevent.
#[test]
fn the_wrapper_catches_automatic_semicolon_insertion() {
    // A snippet whose expression starts on the next line: `return` then a newline is a
    // bare `return;` in JavaScript. It still *parses*, so the syntax check alone cannot
    // catch it — this asserts the shape instead.
    let bad = "return \n(function(){ return 1; })()";
    let wrapped = wrap_like_runtime(bad);
    assert!(
        wrapped.contains("return \n"),
        "the wrapper must not silently reflow the snippet; the ASI hazard has to stay \
         visible so `no_snippet_returns_across_a_newline` can find it"
    );
}

/// The ASI check proper: no snippet may put `return` at the end of a line.
///
/// `node --check` cannot catch this, because `return;` followed by an expression
/// statement is valid JavaScript — it just does the wrong thing. So it is checked
/// structurally, over the `js/` files *and* any inline leftovers. The `js/` half is the
/// live one now that nothing is inline.
#[test]
fn no_snippet_returns_across_a_newline() {
    let mut offenders = Vec::new();
    for s in all_snippets() {
        for (i, line) in s.code.lines().enumerate() {
            let t = line.trim_end();
            // `return` as the last token on a line, with the value on the next.
            if t.ends_with("return") || t.ends_with("return ") {
                offenders.push(format!("{}:{} (snippet line {})", s.file, s.line, i + 1));
            }
        }
    }
    for entry in std::fs::read_dir(Path::new("js")).expect("js/ must exist") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|x| x.to_str()) != Some("js") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("readable js file");
        for (i, line) in src.lines().enumerate() {
            // Comment lines are prose, and the `js/` files explain themselves at length —
            // a sentence ending in the word "return" is not an ASI hazard. Only code counts.
            let t = line.trim();
            if t.starts_with("//") || t.starts_with('*') {
                continue;
            }
            if t.ends_with("return") {
                offenders.push(format!("{}:{}", path.display(), i + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a `return` at end of line becomes a bare `return;` via automatic semicolon \
         insertion, silently yielding undefined:\n  {}",
        offenders.join("\n  ")
    );
}

// --- the extracted js/ files -------------------------------------------------

/// Every `js/*.js` file parses on its own.
///
/// Cheaper and stronger than reconstructing a snippet from a Rust string: these are real
/// JavaScript files, so this is the same check an editor or a linter would apply.
#[test]
fn every_extracted_js_file_parses() {
    if !node_available() {
        eprintln!("skipping: node is not installed (CI has it)");
        return;
    }
    let dir = Path::new("js");
    let files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .expect("js/ must exist")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("js"))
        .collect();
    assert!(
        files.len() >= 6,
        "found only {} files in js/; the snippets were extracted there and a check that \
         finds nothing passes while verifying nothing",
        files.len()
    );
    for file in &files {
        let out = std::process::Command::new("node")
            .arg("--check")
            .arg(file)
            .output()
            .expect("run node --check");
        assert!(
            out.status.success(),
            "{} does not parse:\n{}",
            file.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    eprintln!("checked {} extracted JS files", files.len());
}

/// The check that was missing, and whose absence broke every live test.
///
/// A `js/*.js` file parses fine on its own while still being broken in the form it actually
/// reaches the browser: `page::js` prefixes `return `, so a file starting with its own
/// header comment yields `return // header…`, automatic semicolon insertion closes the
/// statement, and the expression is never evaluated. `node --check` on the bare file cannot
/// see that. This checks the WRAPPED form — what Chrome is really handed.
///
/// "Wrapped" is not one shape. An expression snippet arrives as `return <expr>` inside an
/// async IIFE; a statement snippet arrives as written, wrapped only if it happens to
/// contain a `return `. `js::as_evaluated` wraps it according to the form the snippet declares, so
/// each snippet is checked as the code Chrome actually parses rather than as a shape it was
/// forced into.
#[test]
fn every_extracted_snippet_parses_in_the_form_that_reaches_the_browser() {
    if !node_available() {
        eprintln!("skipping: node is not installed");
        return;
    }
    let tmp = std::env::temp_dir().join(format!("nb-wrapped-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("temp dir");

    let mut failures = Vec::new();
    let mut checked = 0;
    for (name, form, source) in neobrowser::js::all_snippets_for_test() {
        // Placeholders filled with `null` so the structure is checked without needing real
        // arguments, then wrapped the way `page::js` will wrap it.
        let filled = substitute_placeholders(&source);
        let wrapped = neobrowser::js::as_evaluated(&filled, form);

        // The ASI check first: it is a *semantic* fault that still parses, so a syntax
        // check alone would pass a broken snippet.
        if wrapped.contains("return //") || wrapped.contains("return \n") {
            failures.push(format!(
                "{name}: `return` is followed by a comment or newline"
            ));
            continue;
        }
        // An expression snippet must actually be wrapped — if `page::js` left it alone, its
        // top-level `return` is a syntax error in the page, which is the one failure mode
        // this shape check can catch and `node --check` on the bare file cannot.
        if form == neobrowser::js::Form::Expression && wrapped == filled {
            failures.push(format!(
                "{name}: declared an expression but `page::js` would not wrap it, so its \
                 `return` is illegal at top level"
            ));
            continue;
        }
        let file = tmp.join(format!("{name}.js"));
        std::fs::write(&file, &wrapped).expect("write");
        let out = std::process::Command::new("node")
            .arg("--check")
            .arg(&file)
            .output()
            .expect("run node --check");
        if !out.status.success() {
            failures.push(format!(
                "{name}: {}",
                String::from_utf8_lossy(&out.stderr)
                    .lines()
                    .find(|l| l.contains("SyntaxError"))
                    .unwrap_or("syntax error")
                    .trim()
            ));
        }
        checked += 1;
    }
    let _ = std::fs::remove_dir_all(&tmp);
    assert!(
        failures.is_empty(),
        "snippets are broken in the form that reaches the browser:\n  {}",
        failures.join("\n  ")
    );
    // A floor for the same reason as elsewhere in this file: a shipped-snippet list that
    // silently emptied would pass every assertion above while verifying nothing.
    assert!(
        checked >= 22,
        "only {checked} snippets were checked in their wrapped form; \
         `js::all_snippets_for_test` has lost entries"
    );
    eprintln!("checked {checked} snippets in the form that reaches the browser");
}

/// Every `.js` file in `js/` is reachable through `js::all_snippets_for_test`.
///
/// A file nobody registers is a file whose wrapped form is never checked — and the wrapped
/// form is where the ASI bug lives. So an unregistered snippet fails here rather than
/// shipping unverified.
#[test]
fn every_js_file_is_registered_for_the_wrapped_check() {
    // Snippets loaded outside the `Snippet` machinery, with the reason each one is.
    // They are `include_str!` + `str::replace` call sites, so there is no `Snippet` to
    // register; they are still covered by `every_extracted_js_file_parses` above.
    const UNREGISTERED: &[(&str, &str)] = &[
        ("analyze.js", "passed to page::js whole, no placeholders"),
        ("page_info.js", "passed to page::js whole, no placeholders"),
        ("dismiss_overlay.js", "substitutes a bare FORCE token"),
        (
            "dismiss_consent.js",
            "search-module snippet, no placeholders",
        ),
        (
            "stealth.js",
            "injected via Page.addScriptToEvaluateOnNewDocument",
        ),
        ("search_bing_images.js", "search-module snippet"),
        ("search_ddg_text.js", "search-module snippet"),
        ("search_google_text.js", "search-module snippet"),
        ("search_images_extract.js", "search-module snippet"),
        ("search_videos_extract.js", "search-module snippet"),
        ("search_youtube_videos.js", "search-module snippet"),
    ];

    let registered: Vec<String> = neobrowser::js::all_snippets_for_test()
        .into_iter()
        .map(|(name, _, _)| format!("{name}.js"))
        .collect();
    let mut missing = Vec::new();
    for entry in std::fs::read_dir(Path::new("js")).expect("js/ must exist") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|x| x.to_str()) != Some("js") {
            continue;
        }
        let file = path
            .file_name()
            .expect("file name")
            .to_string_lossy()
            .into_owned();
        if registered.contains(&file) || UNREGISTERED.iter().any(|(f, _)| *f == file) {
            continue;
        }
        missing.push(file);
    }
    assert!(
        missing.is_empty(),
        "these js/ files are not registered in js::all_snippets_for_test, so the form that \
         reaches the browser is never checked for them: {missing:?}. Register them, or add \
         them to UNREGISTERED here with the reason."
    );
    // The other direction: a stale UNREGISTERED entry would silently excuse a file that no
    // longer exists, and later excuse a new file that happens to reuse the name.
    for (file, _) in UNREGISTERED {
        assert!(
            Path::new("js").join(file).exists(),
            "UNREGISTERED lists {file}, which no longer exists"
        );
    }
}

/// Each loader loads the file its name promises.
///
/// `include_str!` resolves relative to the *Rust* file, so grouping the loaders into
/// `src/js/*.rs` rewrote all 22 paths at once — and a path that points at the wrong sibling
/// still compiles and still runs. Two snippets with the same placeholder set (the two
/// `debug_capture_*`, the two `login_*` fills) would swap silently and behave plausibly:
/// the console interceptor would uninstall on `start`. So the name-to-file mapping is
/// asserted directly, which also catches the reverse mistake of editing a `.js` file that
/// nothing loads.
#[test]
fn every_loader_loads_the_file_its_name_promises() {
    let mut wrong = Vec::new();
    for (name, _, code) in neobrowser::js::all_snippets_for_test() {
        let path = Path::new("js").join(format!("{name}.js"));
        let Ok(file) = std::fs::read_to_string(&path) else {
            wrong.push(format!("{name}: js/{name}.js does not exist"));
            continue;
        };
        // `code` is post-`expr()`: the header comment and surrounding blank lines are gone,
        // so compare against the same normalisation rather than the raw file.
        let body: String = file
            .lines()
            .skip_while(|l| {
                let t = l.trim();
                t.is_empty() || t.starts_with("//")
            })
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
        let loaded = code.strip_prefix("return ").unwrap_or(&code);
        if loaded != body {
            wrong.push(format!("{name}: loader does not return js/{name}.js"));
        }
    }
    assert!(
        wrong.is_empty(),
        "loader/file mismatch — an include_str! path points at the wrong snippet:\n  {}",
        wrong.join("\n  ")
    );
}

/// Fill `__NAME__` placeholders with `null`, which is syntactically valid everywhere the
/// real arguments (strings, numbers, JSON) appear.
///
/// Names come from the snippets themselves rather than a hand-kept list: a placeholder
/// nobody listed would survive into the checked source as `__SEL__`, which parses as an
/// identifier, so the check would quietly pass on code the browser cannot run.
fn substitute_placeholders(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // `__NAME__` where NAME is upper-case, digits or `_`.
        if chars[i] == '_' && chars.get(i + 1) == Some(&'_') {
            let mut j = i + 2;
            while j < chars.len()
                && (chars[j].is_ascii_uppercase() || chars[j].is_ascii_digit() || chars[j] == '_')
            {
                // Stop at the closing `__`.
                if chars[j] == '_' && chars.get(j + 1) == Some(&'_') {
                    break;
                }
                j += 1;
            }
            if j > i + 2 && chars.get(j) == Some(&'_') && chars.get(j + 1) == Some(&'_') {
                out.push_str("null");
                i = j + 2;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// The substituter must handle every placeholder shape the snippets actually use, and
/// leave the page's own `__`-prefixed globals alone.
#[test]
fn the_substituter_replaces_placeholders_and_spares_page_globals() {
    assert_eq!(
        substitute_placeholders("f(__SEL__, __NTH__)"),
        "f(null, null)"
    );
    assert_eq!(substitute_placeholders("var v = __V__;"), "var v = null;");
    assert_eq!(
        substitute_placeholders("window.__nbClickTarget = t;"),
        "window.__nbClickTarget = t;"
    );
    assert_eq!(
        substitute_placeholders("window.__neo_debug_logs.push(x)"),
        "window.__neo_debug_logs.push(x)"
    );
    // Nothing left behind: a surviving `__NAME__` parses as an identifier, so the syntax
    // check would pass on code the browser cannot run.
    for (name, _, source) in neobrowser::js::all_snippets_for_test() {
        let filled = substitute_placeholders(&source);
        assert!(
            !filled.contains("__SEL__") && !filled.contains("__VAL__"),
            "{name}: a placeholder survived substitution"
        );
    }
}
