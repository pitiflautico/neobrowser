//! Upload and download: the tools that move files between the machine and a page.
//!
//! Both are gated on the same allowlist (`resolve_upload_path`) so a second file-reading
//! tool cannot end up with weaker validation than the first — which is exactly what
//! nearly happened when `har_import` was added.
//!
//! Split by direction: [`upload`] sends a local file into a page and refuses the ones that
//! should not leave the machine, [`download`] receives one.

pub mod download;
pub mod upload;

pub use download::download;
pub use upload::{resolve_upload_path, upload, upload_roots_for_report};
