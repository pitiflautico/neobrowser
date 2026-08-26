//! PDF text extraction: download the current PDF and extract its text.
//!
//! Chrome renders PDFs in its own viewer, so the text never reaches the DOM.
//! The pragmatic path is to download the file and run it through `pdftotext`
//! (poppler), which is available by default on macOS and most Linux distros,
//! and via `choco install poppler` on Windows.

use std::process::Command;

use crate::cdp::CdpClient;

use super::nav::current_url;

/// Extract text from the current page if it is a PDF.
///
/// Returns `None` if the page is not a PDF, `Some(Err)` if extraction failed,
/// or `Some(Ok(text))` with the extracted text.
pub async fn extract_pdf_text(client: &CdpClient) -> Option<Result<String, String>> {
    let url = current_url(client).await.ok()?;
    if !url.to_ascii_lowercase().ends_with(".pdf") {
        return None;
    }

    // Download the PDF to a temp file.
    let pdf_bytes = match download_pdf(&url).await {
        Ok(b) => b,
        Err(e) => return Some(Err(format!("download failed: {e}"))),
    };

    let tmp_dir = std::env::temp_dir();
    let tmp_pdf = tmp_dir.join(format!("neobrowser-{}.pdf", uuid_simple()));
    if let Err(e) = std::fs::write(&tmp_pdf, pdf_bytes) {
        return Some(Err(format!("write temp file failed: {e}")));
    }

    let out = tmp_dir.join(format!("neobrowser-{}.txt", uuid_simple()));
    let status = Command::new("pdftotext")
        .arg("-layout")
        .arg(&tmp_pdf)
        .arg(&out)
        .status();

    let result = match status {
        Ok(s) if s.success() => match std::fs::read_to_string(&out) {
            Ok(text) => Ok(text),
            Err(e) => Err(format!("read extracted text failed: {e}")),
        },
        Ok(s) => Err(format!("pdftotext exited with {s}")),
        Err(e) => Err(format!("pdftotext not available: {e}. Install poppler (brew install poppler / apt install poppler-utils / choco install poppler).")),
    };

    let _ = std::fs::remove_file(&tmp_pdf);
    let _ = std::fs::remove_file(&out);
    Some(result)
}

async fn download_pdf(url: &str) -> Result<Vec<u8>, reqwest::Error> {
    let resp = reqwest::get(url).await?;
    resp.bytes().await.map(|b| b.to_vec())
}

fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{nanos:x}")
}
