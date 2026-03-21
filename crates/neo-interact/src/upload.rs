//! File upload support for input[type=file] elements and multipart form building.
//!
//! `set_file` validates the target is a file input and sets a synthetic value.
//! `build_multipart` assembles a multipart/form-data body from mixed fields.

use neo_dom::DomEngine;

use crate::resolve::resolve;
use crate::InteractError;

/// A file to upload.
#[derive(Debug, Clone)]
pub struct FileUpload {
    /// Path to the file on disk (informational only).
    pub file_path: String,
    /// File name to use in the multipart body.
    pub file_name: String,
    /// MIME content type (e.g., "image/png").
    pub content_type: String,
    /// Raw file bytes.
    pub data: Vec<u8>,
}

/// A single field in a multipart form submission.
#[derive(Debug, Clone)]
pub struct MultipartField {
    /// Form field name.
    pub name: String,
    /// Field value — text or file.
    pub value: MultipartValue,
}

/// Value of a multipart field.
#[derive(Debug, Clone)]
pub enum MultipartValue {
    /// Plain text value.
    Text(String),
    /// File upload.
    File(FileUpload),
}

/// Set a file on an input[type=file] element.
///
/// Resolves the target, verifies it is `<input type="file">`, sets the
/// file name as a synthetic value attribute, and dispatches a "change" event
/// conceptually. The actual file data must be attached at submission time
/// via `build_multipart`.
pub fn set_file(
    dom: &mut dyn DomEngine,
    target: &str,
    file: &FileUpload,
) -> Result<(), InteractError> {
    let el = resolve(dom, target)?;
    let tag = dom.tag_name(el).unwrap_or_default();

    if tag != "input" {
        return Err(InteractError::TypeMismatch {
            expected: "input[type=file]".to_string(),
            actual: tag,
        });
    }

    let input_type = dom
        .get_attribute(el, "type")
        .unwrap_or_default()
        .to_lowercase();

    if input_type != "file" {
        return Err(InteractError::TypeMismatch {
            expected: "input[type=file]".to_string(),
            actual: format!("input[type={input_type}]"),
        });
    }

    // Set synthetic value (file name) so the DOM reflects the selection.
    dom.set_attribute(el, "value", &file.file_name);

    // In a real runtime, dispatch_event("change", true) would fire here.

    Ok(())
}

/// Build a multipart/form-data body from a slice of fields.
///
/// Returns `(content_type_header, body_bytes)` where the content type
/// includes the generated boundary string.
pub fn build_multipart(fields: &[MultipartField]) -> (String, Vec<u8>) {
    let boundary = generate_boundary();
    let mut body = Vec::new();

    for field in fields {
        // Boundary delimiter
        body.extend_from_slice(b"--");
        body.extend_from_slice(boundary.as_bytes());
        body.extend_from_slice(b"\r\n");

        match &field.value {
            MultipartValue::Text(text) => {
                body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{}\"\r\n\r\n",
                        field.name
                    )
                    .as_bytes(),
                );
                body.extend_from_slice(text.as_bytes());
            }
            MultipartValue::File(file) => {
                body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n\
                         Content-Type: {}\r\n\r\n",
                        field.name, file.file_name, file.content_type
                    )
                    .as_bytes(),
                );
                body.extend_from_slice(&file.data);
            }
        }

        body.extend_from_slice(b"\r\n");
    }

    // Closing boundary
    body.extend_from_slice(b"--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(b"--\r\n");

    let content_type = format!("multipart/form-data; boundary={boundary}");
    (content_type, body)
}

/// Detect content type from a file extension.
///
/// Maps common extensions to MIME types. Returns `application/octet-stream`
/// for unknown extensions.
pub fn detect_content_type(file_name: &str) -> &'static str {
    let ext = file_name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "application/javascript",
        "json" => "application/json",
        "xml" => "application/xml",
        "zip" => "application/zip",
        "csv" => "text/csv",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        _ => "application/octet-stream",
    }
}

/// Generate a random boundary string for multipart encoding.
fn generate_boundary() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    // Mix in the address of a stack variable for extra entropy.
    let stack_var: u8 = 0;
    let addr = std::ptr::addr_of!(stack_var) as usize;

    format!("----NeoRender{:x}{:x}", nanos, addr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_dom::MockDomEngine;

    fn make_file_input(dom: &mut MockDomEngine) -> neo_dom::ElementId {
        let id = dom.add_element("input", &[("type", "file"), ("name", "avatar")], "");
        dom.set_interactive(id, true);
        id
    }

    #[test]
    fn test_build_multipart_text_only() {
        let fields = vec![
            MultipartField {
                name: "username".to_string(),
                value: MultipartValue::Text("alice".to_string()),
            },
            MultipartField {
                name: "email".to_string(),
                value: MultipartValue::Text("alice@example.com".to_string()),
            },
        ];

        let (content_type, body) = build_multipart(&fields);
        let body_str = String::from_utf8_lossy(&body);

        assert!(content_type.starts_with("multipart/form-data; boundary="));
        assert!(body_str.contains("Content-Disposition: form-data; name=\"username\""));
        assert!(body_str.contains("alice"));
        assert!(body_str.contains("Content-Disposition: form-data; name=\"email\""));
        assert!(body_str.contains("alice@example.com"));
        // Closing boundary
        assert!(body_str.ends_with("--\r\n"));
    }

    #[test]
    fn test_build_multipart_with_file() {
        let fields = vec![
            MultipartField {
                name: "name".to_string(),
                value: MultipartValue::Text("test".to_string()),
            },
            MultipartField {
                name: "photo".to_string(),
                value: MultipartValue::File(FileUpload {
                    file_path: "/tmp/photo.jpg".to_string(),
                    file_name: "photo.jpg".to_string(),
                    content_type: "image/jpeg".to_string(),
                    data: vec![0xFF, 0xD8, 0xFF, 0xE0],
                }),
            },
        ];

        let (content_type, body) = build_multipart(&fields);
        let body_str = String::from_utf8_lossy(&body);

        assert!(content_type.starts_with("multipart/form-data; boundary="));
        assert!(body_str.contains("Content-Disposition: form-data; name=\"photo\"; filename=\"photo.jpg\""));
        assert!(body_str.contains("Content-Type: image/jpeg"));
        // Binary data should be in the body
        assert!(body.windows(4).any(|w| w == [0xFF, 0xD8, 0xFF, 0xE0]));
    }

    #[test]
    fn test_content_type_detection() {
        assert_eq!(detect_content_type("photo.jpg"), "image/jpeg");
        assert_eq!(detect_content_type("photo.jpeg"), "image/jpeg");
        assert_eq!(detect_content_type("image.png"), "image/png");
        assert_eq!(detect_content_type("doc.pdf"), "application/pdf");
        assert_eq!(detect_content_type("readme.txt"), "text/plain");
        assert_eq!(detect_content_type("data.bin"), "application/octet-stream");
        assert_eq!(detect_content_type("noext"), "application/octet-stream");
    }

    #[test]
    fn test_multipart_boundary_unique() {
        let (ct1, _) = build_multipart(&[]);
        // Small delay not needed — nanos + stack addr should differ.
        let (ct2, _) = build_multipart(&[]);

        let b1 = ct1.strip_prefix("multipart/form-data; boundary=").unwrap();
        let b2 = ct2.strip_prefix("multipart/form-data; boundary=").unwrap();
        assert_ne!(b1, b2, "boundaries should be unique across calls");
    }

    #[test]
    fn test_set_file_wrong_type() {
        let mut dom = MockDomEngine::new();
        // Add a text input, not a file input
        dom.add_element("input", &[("type", "text"), ("name", "username")], "");

        let file = FileUpload {
            file_path: "/tmp/photo.jpg".to_string(),
            file_name: "photo.jpg".to_string(),
            content_type: "image/jpeg".to_string(),
            data: vec![0xFF, 0xD8],
        };

        let result = set_file(&mut dom, "input", &file);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("type mismatch"),
            "expected TypeMismatch error, got: {err}"
        );
    }

    #[test]
    fn test_set_file_success() {
        let mut dom = MockDomEngine::new();
        make_file_input(&mut dom);

        let file = FileUpload {
            file_path: "/tmp/photo.jpg".to_string(),
            file_name: "photo.jpg".to_string(),
            content_type: "image/jpeg".to_string(),
            data: vec![0xFF, 0xD8],
        };

        let result = set_file(&mut dom, "input", &file);
        assert!(result.is_ok());

        // Value attribute should be set to the file name
        let val = dom.get_attribute(0, "value");
        assert_eq!(val.as_deref(), Some("photo.jpg"));
    }

    #[test]
    fn test_set_file_not_input() {
        let mut dom = MockDomEngine::new();
        dom.add_element("div", &[], "some div");

        let file = FileUpload {
            file_path: "/tmp/f.txt".to_string(),
            file_name: "f.txt".to_string(),
            content_type: "text/plain".to_string(),
            data: vec![],
        };

        let result = set_file(&mut dom, "some div", &file);
        assert!(result.is_err());
    }
}
