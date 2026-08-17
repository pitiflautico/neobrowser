//! JavaScript that runs inside the page, kept as real `.js` files.
//!
//! Every snippet used to live in a Rust `format!` string. That cost us a genuine bug: the
//! state digest was built as `format!("return {STATE_JS}")` where the snippet began with a
//! newline, so the generated code read `return\n(function…)` — JavaScript's automatic
//! semicolon insertion turned it into a bare `return;`. Every observation returned
//! `undefined`, so every verified action reported `uncertain`. The Rust compiled, the unit
//! tests passed, and it only surfaced when driving a real page.
//!
//! Three things change by moving the snippets to `js/`:
//!
//! - **They are syntax-checked.** `js/*.js` is valid JavaScript, so `node --check` (in
//!   `tests/embedded_js.rs`) verifies it directly instead of reconstructing it from a Rust
//!   string.
//! - **The `format!` escaping is gone.** A JS object literal no longer needs `{{`/`}}`,
//!   which was the other silent source of brace mismatches.
//! - **They are readable.** Editors highlight and lint them; a reviewer sees JavaScript
//!   rather than a quoted blob.
//!
//! Parameters use `__NAME__` placeholders substituted by [`Snippet::with`], not `format!`.
//! A placeholder is inert JavaScript-wise, so the file still parses on its own — which is
//! precisely what makes the syntax check meaningful.

/// A JS snippet with `__NAME__` placeholders.
pub struct Snippet {
    source: String,
}

impl Snippet {
    pub fn new(source: &str) -> Self {
        Self {
            source: source.to_string(),
        }
    }

    /// Substitute one placeholder. Values must already be JSON-encoded when they stand in
    /// for a string or object — `with("SEL", &serde_json::to_string(sel)?)` — so a
    /// selector containing a quote cannot break out of its literal.
    pub fn with(mut self, name: &str, value: &str) -> Self {
        self.source = self.source.replace(&format!("__{name}__"), value);
        self
    }

    /// The snippet as an expression to evaluate.
    ///
    /// Leading blank lines AND leading `//` comment lines are stripped, and this is
    /// load-bearing rather than cosmetic. `page::js` prefixes `return `, so a snippet that
    /// begins with a comment produces `return // …` — the rest of that line is a comment,
    /// automatic semicolon insertion closes the statement, and the expression below it is
    /// never returned. The result is `undefined`, silently.
    ///
    /// That is the same failure that once made every verified action report `uncertain`,
    /// and moving the snippets into documented `.js` files reintroduced it verbatim: the
    /// file header comments are exactly what lands after `return`. So the header is
    /// stripped here, where every caller benefits, rather than asking each `.js` file to
    /// forgo its own documentation.
    pub fn expr(&self) -> String {
        self.source
            .lines()
            .skip_while(|l| {
                let t = l.trim();
                t.is_empty() || t.starts_with("//")
            })
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string()
    }

    /// The snippet wrapped as `return <expr>`, for `page::js`.
    pub fn returning(&self) -> String {
        format!("return {}", self.expr())
    }

    /// Any placeholder left unsubstituted. A snippet reaching the browser with a literal
    /// `__SEL__` in it would fail in a way that looks like a page problem, so callers can
    /// assert this instead.
    pub fn unresolved(&self) -> Vec<String> {
        let mut out = Vec::new();
        let bytes: Vec<char> = self.source.chars().collect();
        let mut i = 0;
        while i + 3 < bytes.len() {
            if bytes[i] == '_' && bytes[i + 1] == '_' {
                if let Some(end) = (i + 2..bytes.len().saturating_sub(1))
                    .find(|&j| bytes[j] == '_' && bytes.get(j + 1) == Some(&'_'))
                {
                    let name: String = bytes[i + 2..end].iter().collect();
                    if !name.is_empty()
                        && name
                            .chars()
                            .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
                    {
                        out.push(name);
                        i = end + 2;
                        continue;
                    }
                }
            }
            i += 1;
        }
        out
    }
}

/// The page-state digest used by every verified action.
pub fn state_digest() -> Snippet {
    Snippet::new(include_str!("../js/state_digest.js"))
}

/// Shadow-DOM and same-origin-iframe piercing.
pub fn pierce() -> Snippet {
    Snippet::new(include_str!("../js/pierce.js"))
}

/// Web Vitals and navigation timing.
pub fn vitals() -> Snippet {
    Snippet::new(include_str!("../js/vitals.js"))
}

/// Frame reachability, for `list_frames`.
pub fn frame_access() -> Snippet {
    Snippet::new(include_str!("../js/frame_access.js"))
}

/// Resolved CSS for one element, plus why it is invisible when it is.
pub fn computed_style() -> Snippet {
    Snippet::new(include_str!("../js/computed_style.js"))
}

/// Set a checkbox, radio, select or contenteditable through the framework-visible setter.
pub fn set_control() -> Snippet {
    Snippet::new(include_str!("../js/set_control.js"))
}

/// Every shipped snippet as `(name, returning_form)`, for the integration test that
/// verifies the form actually handed to Chrome.
///
/// Public because `tests/embedded_js.rs` is a separate binary and cannot reach a
/// `#[cfg(test)]` item — and the check it performs (does this parse *wrapped*) is the one
/// whose absence let a broken refactor through.
pub fn all_snippets_for_test() -> Vec<(&'static str, String)> {
    vec![
        ("state_digest", state_digest().returning()),
        ("pierce", pierce().returning()),
        ("vitals", vitals().returning()),
        ("frame_access", frame_access().returning()),
        ("computed_style", computed_style().returning()),
        ("set_control", set_control().returning()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_substituted_and_reported() {
        let s = Snippet::new("f(__SEL__, __N__)");
        assert_eq!(s.unresolved(), vec!["SEL", "N"]);
        let done = s.with("SEL", "\"#a\"").with("N", "3");
        assert_eq!(done.expr(), "f(\"#a\", 3)");
        assert!(done.unresolved().is_empty());
    }

    /// The whole point of the module: whatever precedes the expression in the file, the
    /// expression itself must land immediately after `return`.
    #[test]
    fn returning_puts_the_expression_directly_after_return() {
        // Leading blank lines.
        let out = Snippet::new("\n\n(function(){ return 1; })()\n").returning();
        assert!(out.starts_with("return (function"), "got {out:?}");

        // Leading comment lines — the case that broke every live test after the snippets
        // were moved into documented .js files.
        let out =
            Snippet::new("// a header\n//\n// more prose\n(function(){ return 1; })()").returning();
        assert!(
            out.starts_with("return (function"),
            "a file header must not end up after `return`, got {out:?}"
        );
        assert!(!out.contains("return //"), "ASI hazard: {out:?}");

        // A comment INSIDE the expression is untouched — only the header is stripped.
        let out = Snippet::new("// header\n(function(){ // inner\n return 1; })()").returning();
        assert!(
            out.contains("// inner"),
            "inner comments must survive: {out:?}"
        );
    }

    /// Every shipped snippet, in the form it actually reaches the browser, must have its
    /// expression on the same line as `return`.
    #[test]
    fn no_shipped_snippet_has_the_asi_hazard() {
        for (name, snippet) in [
            ("state_digest", state_digest()),
            ("pierce", pierce()),
            ("vitals", vitals()),
            ("frame_access", frame_access()),
            ("computed_style", computed_style()),
            ("set_control", set_control()),
        ] {
            let wrapped = snippet.returning();
            let first = wrapped.lines().next().unwrap_or("");
            assert!(
                first.starts_with("return (") || first.starts_with("return ("),
                "{name}: `return` is not followed by the expression on the same line: {first:?}"
            );
            assert!(!wrapped.contains("return //"), "{name}: ASI hazard");
        }
    }

    /// Every shipped snippet loads and has no placeholder that callers forgot to name.
    /// A snippet whose placeholders are undocumented is one that will reach the browser
    /// half-substituted.
    #[test]
    fn every_snippet_loads_and_declares_its_placeholders() {
        let cases: Vec<(&str, Snippet, Vec<&str>)> = vec![
            ("state_digest", state_digest(), vec!["SALT"]),
            ("pierce", pierce(), vec!["SEL", "ACTION", "VALUE"]),
            ("vitals", vitals(), vec![]),
            ("frame_access", frame_access(), vec![]),
            ("computed_style", computed_style(), vec!["SEL", "PROPS"]),
            ("set_control", set_control(), vec!["SEL", "VALUE"]),
        ];
        for (name, snippet, expected) in cases {
            assert!(!snippet.expr().is_empty(), "{name} is empty");
            let mut found = snippet.unresolved();
            found.sort();
            found.dedup();
            let mut want: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
            want.sort();
            assert_eq!(found, want, "{name} placeholders");
        }
    }
}
