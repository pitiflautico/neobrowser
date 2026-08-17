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
//! Embedded JS has no compiler. This test is the compiler: it extracts each snippet,
//! undoes the `format!` escaping, and runs `node --check` over it. It catches the ASI
//! class of bug, mismatched braces from `{{`/`}}` escaping, and anything else that is a
//! syntax error — for all ~29 snippets at once, including ones added later.
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

#[test]
fn every_embedded_js_snippet_parses() {
    if !node_available() {
        eprintln!("skipping: node is not installed (CI has it)");
        return;
    }

    let snippets = all_snippets();

    // A floor, not a target. It exists because a broken extractor that finds nothing would
    // pass every assertion below while verifying nothing — so this is what makes the rest
    // of the test mean anything.
    //
    // The number goes DOWN as snippets migrate to `js/*.js`, where they are checked
    // directly and more strictly by `every_extracted_js_file_parses`. Lower it deliberately
    // when you move one out; do not raise it to make a failure go away.
    assert!(
        snippets.len() >= 12,
        "found only {} JS snippets still inline in Rust. If snippets were just extracted to \
         js/, lower this floor in the same change. If not, the extractor is broken",
        snippets.len()
    );

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
/// structurally.
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
#[test]
fn every_extracted_snippet_parses_in_the_form_that_reaches_the_browser() {
    if !node_available() {
        eprintln!("skipping: node is not installed");
        return;
    }
    let tmp = std::env::temp_dir().join(format!("nb-wrapped-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("temp dir");

    let mut failures = Vec::new();
    for (name, source) in neobrowser::js::all_snippets_for_test() {
        // The exact string `page::js` receives, with placeholders filled by `null` so the
        // structure is checked without needing real arguments.
        let wrapped = format!(
            "(async function(){{{}}})()",
            substitute_placeholders(&source)
        );
        // The ASI check first: it is a *semantic* fault that still parses, so a syntax
        // check alone would pass a broken snippet.
        if wrapped.contains("return //") || wrapped.contains("return \n") {
            failures.push(format!(
                "{name}: `return` is followed by a comment or newline"
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
    }
    let _ = std::fs::remove_dir_all(&tmp);
    assert!(
        failures.is_empty(),
        "snippets are broken in the form that reaches the browser:\n  {}",
        failures.join("\n  ")
    );
}

/// Fill `__NAME__` placeholders with `null`, which is syntactically valid everywhere the
/// real arguments (strings, numbers, JSON) appear.
fn substitute_placeholders(source: &str) -> String {
    let mut out = source.to_string();
    for name in ["SALT", "SEL", "ACTION", "VALUE", "PROPS"] {
        out = out.replace(&format!("__{name}__"), "null");
    }
    out
}
