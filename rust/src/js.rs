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
//!
//! # Layout
//!
//! This file holds what is true of snippets in general: the [`Snippet`] type, the [`Form`] a
//! snippet reaches the browser in, and [`all_snippets_for_test`] — the inventory every check
//! iterates. The loaders themselves are grouped one module per domain ([`forms`],
//! [`harvest`], [`inspect`], [`login`], [`page`]) and re-exported flat, so a call site still
//! writes `js::fill_control()`.
//!
//! The grouping is a filing convenience, not a claim about the snippets: [`Form`] is the
//! distinction that actually changes how a snippet must be used, and it cuts across the
//! groups.

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

pub mod forms;
pub mod harvest;
pub mod inspect;
pub mod login;
pub mod page;

// Re-exported flat, because a snippet's group is a filing decision and not part of its
// identity: `js::fill_control()` reads better at a call site than `js::forms::fill_control()`,
// and moving a snippet between groups then costs nothing at the call sites.
pub use forms::{fill_control, find_and_click, form_fill_fields, submit_form};
pub use harvest::{extract_links, extract_table, paginate_click, paginate_next};
pub use inspect::{computed_style, debug_capture_off, debug_capture_on, fetch_source_map, vitals};
pub use login::{login_fill_field, login_find_field, login_state, login_submit};
pub use page::{frame_access, pierce, read_with_links, set_control, state_digest, wall_signals};

/// How a snippet is handed to `page::js`, which is not uniform and cannot be made so.
///
/// The distinction decides what "correct" even means for a snippet, so it is recorded
/// rather than inferred: an [`Expression`](Form::Expression) with its value dropped on the
/// floor and a [`Statements`](Form::Statements) snippet look identical from the Rust side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// The call site passes [`Snippet::returning`] to [`crate::page::eval_body`], which
    /// wraps it in an async IIFE. The expression MUST begin on the same line as `return` —
    /// that is the ASI hazard this module exists to prevent.
    Expression,
    /// The call site passes [`Snippet::expr`] to [`crate::page::eval_expr`]: a statement
    /// sequence evaluated for its effect on the page, whose value the call site must not
    /// use.
    Statements,
}

/// The exact string Chrome will parse for `code`, given the form the snippet declares.
///
/// No heuristic: the form is declared once, per snippet, in [`all_snippets_for_test`], and
/// both the runtime and this function follow it. `page::js` used to guess by looking for
/// `return ` in the text, which meant this helper had to duplicate the guess and stay in
/// sync with it. Splitting that function into [`crate::page::eval_body`] and
/// [`crate::page::eval_expr`] removed the guess from every call site this crate owns, so
/// there is nothing left here to mirror.
///
/// `tests/embedded_js.rs` uses this to `node --check` the wrapped form, which is the check
/// whose absence let a broken refactor through: the bare `.js` file parsed fine while the
/// string actually handed to Chrome did not.
pub fn as_evaluated(code: &str, form: Form) -> String {
    match form {
        Form::Expression => format!("(async function(){{{code}}})()"),
        Form::Statements => code.to_string(),
    }
}

/// Every shipped snippet as `(name, form, code)`, where `code` is the exact string its
/// call site hands to `page::js` — `returning()` for an expression, `expr()` for
/// statements.
///
/// Public because `tests/embedded_js.rs` is a separate binary and cannot reach a
/// `#[cfg(test)]` item — and the check it performs (does this parse *in the form Chrome
/// receives*) is the one whose absence let a broken refactor through.
pub fn all_snippets_for_test() -> Vec<(&'static str, Form, String)> {
    use Form::{Expression, Statements};
    vec![
        ("state_digest", Expression, state_digest().returning()),
        ("pierce", Expression, pierce().returning()),
        ("vitals", Expression, vitals().returning()),
        ("frame_access", Expression, frame_access().returning()),
        ("computed_style", Expression, computed_style().returning()),
        ("set_control", Expression, set_control().returning()),
        ("fill_control", Expression, fill_control().returning()),
        (
            "form_fill_fields",
            Expression,
            form_fill_fields().returning(),
        ),
        ("submit_form", Expression, submit_form().returning()),
        ("find_and_click", Expression, find_and_click().returning()),
        ("extract_links", Expression, extract_links().returning()),
        ("extract_table", Expression, extract_table().returning()),
        ("paginate_click", Expression, paginate_click().returning()),
        ("paginate_next", Expression, paginate_next().returning()),
        ("login_state", Expression, login_state().returning()),
        (
            "fetch_source_map",
            Expression,
            fetch_source_map().returning(),
        ),
        ("wall_signals", Expression, wall_signals().returning()),
        ("read_with_links", Expression, read_with_links().returning()),
        ("debug_capture_on", Statements, debug_capture_on().expr()),
        ("debug_capture_off", Statements, debug_capture_off().expr()),
        ("login_find_field", Statements, login_find_field().expr()),
        ("login_fill_field", Statements, login_fill_field().expr()),
        ("login_submit", Statements, login_submit().expr()),
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
    ///
    /// Checked per [`Form`] rather than uniformly: only an expression snippet is prefixed
    /// with `return`, so demanding `return (` of every snippet would either be a lie about
    /// the statement ones or force them into a shape they do not have.
    #[test]
    fn no_shipped_snippet_has_the_asi_hazard() {
        for (name, form, code) in all_snippets_for_test() {
            if form == Form::Expression {
                let first = code.lines().next().unwrap_or("");
                let after = first.strip_prefix("return ").unwrap_or_else(|| {
                    panic!("{name}: an expression snippet must start with `return `: {first:?}")
                });
                assert!(
                    !after.trim().is_empty() && !after.trim_start().starts_with("//"),
                    "{name}: `return` is not followed by the expression on the same line: \
                     {first:?}"
                );
            }
            assert!(!code.contains("return //"), "{name}: ASI hazard");
            for (i, line) in code.lines().enumerate() {
                assert!(
                    line.trim_end() != "return",
                    "{name}: line {} is a bare `return` at end of line; automatic semicolon \
                     insertion makes it `return;` and the value below is never returned",
                    i + 1
                );
            }
        }
    }

    /// A statement snippet whose only `return` sits in a nested callback is wrapped by
    /// `page::js` as a function body that returns nothing. That is fine — but only because
    /// no call site reads the value, and this records which snippets that applies to so the
    /// next person to want a value back knows they cannot just take one.
    #[test]
    fn statement_snippets_are_the_ones_whose_value_is_unused() {
        let statements: Vec<&str> = all_snippets_for_test()
            .into_iter()
            .filter(|(_, form, _)| *form == Form::Statements)
            .map(|(name, _, _)| name)
            .collect();
        assert_eq!(
            statements,
            vec![
                "debug_capture_on",
                "debug_capture_off",
                "login_find_field",
                "login_fill_field",
                "login_submit",
            ],
            "the set of snippets evaluated for effect changed; confirm the new one's return \
             value really is unused at its call site before updating this list"
        );
    }

    /// The declared form decides the wrapping, and the text of the code does not.
    ///
    /// This replaces a test that asserted the old heuristic and its mirror agreed, including
    /// the case where `return;` — no trailing space — was left unwrapped. That test verified
    /// a guess was consistently wrong in the same way. The property worth holding is the
    /// opposite one: identical code wraps differently depending only on what its call site
    /// declared, so no future snippet can be misclassified by how it happens to be written.
    #[test]
    fn the_declared_form_decides_the_wrapping_not_the_text() {
        // Same text, both forms — the only difference is the declaration.
        assert_eq!(
            as_evaluated("return 1", Form::Expression),
            "(async function(){return 1})()"
        );
        assert_eq!(as_evaluated("return 1", Form::Statements), "return 1");

        // Text that would have fooled the old heuristic: a `return` only inside a callback.
        // Under the heuristic this was wrapped into a body that returned nothing, silently
        // yielding `undefined`. Now it is wrapped only if its call site says it is a body.
        let nested = "xs.map(function(x){ return x; });";
        assert_eq!(as_evaluated(nested, Form::Statements), nested);
        assert_eq!(
            as_evaluated(nested, Form::Expression),
            format!("(async function(){{{nested}}})()")
        );
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
            ("fill_control", fill_control(), vec!["SEL", "VAL"]),
            (
                "form_fill_fields",
                form_fill_fields(),
                vec!["IDX", "LABEL", "VAL"],
            ),
            ("submit_form", submit_form(), vec![]),
            (
                "find_and_click",
                find_and_click(),
                vec!["NTH", "ROLE", "TEXTQ", "TEXTRAW"],
            ),
            ("extract_links", extract_links(), vec![]),
            ("extract_table", extract_table(), vec!["IDX", "SEL"]),
            ("paginate_click", paginate_click(), vec!["SEL"]),
            ("paginate_next", paginate_next(), vec![]),
            ("debug_capture_on", debug_capture_on(), vec![]),
            ("debug_capture_off", debug_capture_off(), vec![]),
            ("login_find_field", login_find_field(), vec!["V"]),
            ("login_fill_field", login_fill_field(), vec!["V"]),
            ("login_submit", login_submit(), vec![]),
            ("login_state", login_state(), vec![]),
            ("fetch_source_map", fetch_source_map(), vec!["URL"]),
            ("wall_signals", wall_signals(), vec![]),
            ("read_with_links", read_with_links(), vec!["SELECTOR"]),
        ];
        // Every snippet reachable through `all_snippets_for_test` must be listed, or a
        // half-substituted one ships unnoticed.
        let declared: Vec<&str> = cases.iter().map(|(n, _, _)| *n).collect();
        for (name, _, _) in all_snippets_for_test() {
            assert!(
                declared.contains(&name),
                "{name} ships but its placeholders are undeclared here"
            );
        }
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
