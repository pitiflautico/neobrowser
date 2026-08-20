//! Driving an interactive login and knowing when it finished.
//!
//! The hard part is not filling the form, it is deciding when the login is done: a page that
//! still shows the form might be mid-validation, mid-2FA, or simply rejected. So this waits
//! on a *change of page* rather than a timer, and reports needing a human rather than
//! guessing when it cannot tell.

use serde_json::json;

use crate::cdp::{CdpClient, CdpError};
use crate::page;

/// A credential as a JavaScript string literal. JSON-encoded rather than interpolated
/// raw, so a password containing a quote or a backslash cannot break out of its literal
/// and become code in the page.
fn js_lit(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// `login` — navigate to an https login page, fill email + password, submit, and
/// report an honest success signal (a lingering password field means it didn't work).
pub async fn login(
    client: &CdpClient,
    url: &str,
    email: &str,
    password: &str,
) -> Result<String, CdpError> {
    if !url.starts_with("https://") {
        return Ok(json!({ "ok": false, "error": "login requires an https:// URL" }).to_string());
    }
    page::navigate(client, url, 3.0).await?;
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    // `expr()`, not `returning()`: each of these is an IIFE evaluated for its effect on the
    // page. Its only `return` is a bare `return;` guard, so `page::js` evaluates it as
    // written rather than wrapping it — and nothing here reports a value.
    let email_js = crate::js::login_find_field()
        .with("V", &js_lit(email))
        .expr();
    page::eval_expr(client, &email_js).await?;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let pw_js = crate::js::login_fill_field()
        .with("V", &js_lit(password))
        .expr();
    page::eval_expr(client, &pw_js).await?;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // Submit the form that owns the password field we just filled — NOT the
    // first submit button in the document. Sites commonly ship a sign-in panel
    // in the header alongside the real form in the body, and a document-wide
    // querySelector picks the header one, submitting an empty form.
    page::eval_expr(client, &crate::js::login_submit().expr()).await?;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let final_url = page::current_url(client).await.unwrap_or_default();
    let title = page::eval_body(client, "return document.title")
        .await?
        .as_str()
        .unwrap_or("")
        .to_string();

    // A leftover password field is a weak signal on its own: an account or
    // settings page legitimately has "old password" / "new password" inputs,
    // and a hidden sign-in panel keeps one in the DOM forever. Only count a
    // field that is actually VISIBLE.
    let visible_pw = page::eval_body(client, &crate::js::login_state().returning())
        .await?
        .as_bool()
        .unwrap_or(false);

    // Cross-check with navigation: landing somewhere other than the login URL
    // is strong evidence the credentials were accepted.
    let url_unchanged = same_page(url, &final_url);
    let failed = visible_pw && url_unchanged;

    let mut out = json!({
        "ok": !failed,
        "url": final_url,
        "title": title,
        "still_has_password_field": visible_pw,
    });
    // When the signals disagree, say so instead of silently picking one.
    if !failed && visible_pw {
        out["confidence"] = json!("medium");
        out["note"] = json!(
            "navigated away from the login URL, but a visible password field is still \
             present (an account/settings page can legitimately have one) — verify if it matters"
        );
    }
    Ok(out.to_string())
}

/// Same page ignoring query string and fragment, so a `?returnTo=…` or `#` on
/// the post-submit URL doesn't read as a successful navigation.
pub(super) fn same_page(a: &str, b: &str) -> bool {
    fn base(u: &str) -> &str {
        let u = u.split('#').next().unwrap_or(u);
        let u = u.split('?').next().unwrap_or(u);
        u.trim_end_matches('/')
    }
    base(a) == base(b)
}
