//! Getting structured data out of a page, including across pages.
//!
//! `extract` and `extract_table` turn a rendered DOM back into data, and `paginate` is here
//! rather than with the navigation verbs because it is only ever used in the same loop:
//! extract this page, advance, extract the next. It reports whether it actually advanced,
//! which matters — a "next" link that is present but disabled is the standard way a
//! scraping loop silently re-reads page one forever.

use crate::cdp::{CdpClient, CdpError};
use crate::page;

use super::{js_lit, str_or};

/// `extract` — links (default) or the outerHTML of all tables.
pub async fn extract(client: &CdpClient, what: &str) -> Result<String, CdpError> {
    if what == "links" {
        Ok(str_or(
            page::js(client, &crate::js::extract_links().returning()).await?,
            "[]",
        ))
    } else {
        Ok(str_or(
            page::js(
                client,
                "return Array.from(document.querySelectorAll('table')).map(function(t){ return t.outerHTML; }).join('\\n');",
            )
            .await?,
            "",
        ))
    }
}

/// `extract_table` — parse a table into an array of header→cell objects.
pub async fn extract_table(
    client: &CdpClient,
    selector: &str,
    index: i64,
) -> Result<String, CdpError> {
    let code = crate::js::extract_table()
        .with("SEL", &js_lit(selector))
        .with("IDX", &index.to_string())
        .returning();
    Ok(str_or(page::js(client, &code).await?, "[]"))
}

/// `paginate` — click a "next" control (given selector or auto-detected), then frame.
pub async fn paginate(client: &CdpClient, selector: Option<&str>) -> Result<String, CdpError> {
    let result = if let Some(sel) = selector {
        let code = crate::js::paginate_click()
            .with("SEL", &js_lit(sel))
            .returning();
        str_or(page::js(client, &code).await?, r#"{"ok": false}"#)
    } else {
        str_or(
            page::js(client, &crate::js::paginate_next().returning()).await?,
            r#"{"ok": false}"#,
        )
    };
    page::nudge_frame(client).await;
    Ok(result)
}
