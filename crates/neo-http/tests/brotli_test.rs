//! Tests for manual brotli decompression used in RquestClient.

use std::io::Write;

#[test]
fn brotli_roundtrip_short_text() {
    let original = "Hello World! This is a test of brotli decompression.";

    let mut compressed = Vec::new();
    {
        let mut encoder = brotli::CompressorWriter::new(&mut compressed, 4096, 11, 22);
        encoder.write_all(original.as_bytes()).unwrap();
        // Drop flushes the encoder
    }

    assert!(!compressed.is_empty(), "compressed output should not be empty");
    assert!(
        compressed.len() < original.len(),
        "compressed should be smaller than original for non-trivial text"
    );

    let mut decompressed = Vec::new();
    let mut reader = brotli::Decompressor::new(&compressed[..], 4096);
    std::io::Read::read_to_end(&mut reader, &mut decompressed).unwrap();

    assert_eq!(
        String::from_utf8_lossy(&decompressed),
        original,
        "decompressed text must match original"
    );
}

#[test]
fn brotli_roundtrip_html_payload() {
    let html = r#"<!DOCTYPE html>
<html>
<head><title>Test Page</title></head>
<body>
    <div id="app">
        <script src="/vendor.j304wec9xx.js"></script>
        <script type="module" src="/vendor.j304wec9xx.js"></script>
    </div>
</body>
</html>"#;

    let mut compressed = Vec::new();
    {
        let mut encoder = brotli::CompressorWriter::new(&mut compressed, 4096, 6, 22);
        encoder.write_all(html.as_bytes()).unwrap();
    }

    let mut decompressed = Vec::new();
    let mut reader = brotli::Decompressor::new(&compressed[..], 4096);
    std::io::Read::read_to_end(&mut reader, &mut decompressed).unwrap();

    assert_eq!(String::from_utf8_lossy(&decompressed), html);
}

#[test]
fn brotli_roundtrip_empty_input() {
    let original = "";

    let mut compressed = Vec::new();
    {
        let mut encoder = brotli::CompressorWriter::new(&mut compressed, 4096, 11, 22);
        encoder.write_all(original.as_bytes()).unwrap();
    }

    let mut decompressed = Vec::new();
    let mut reader = brotli::Decompressor::new(&compressed[..], 4096);
    std::io::Read::read_to_end(&mut reader, &mut decompressed).unwrap();

    assert_eq!(String::from_utf8_lossy(&decompressed), original);
}

#[test]
fn brotli_invalid_data_fallback() {
    // Random bytes that are not valid brotli — decompression should fail.
    let garbage = b"this is not brotli compressed data at all!!!";

    let mut decompressed = Vec::new();
    let mut reader = brotli::Decompressor::new(&garbage[..], 4096);
    let result = std::io::Read::read_to_end(&mut reader, &mut decompressed);

    // The client code falls back to raw bytes on decompression failure,
    // so we just verify the error is detected (not a panic).
    assert!(
        result.is_err(),
        "invalid brotli data should produce an error"
    );
}
