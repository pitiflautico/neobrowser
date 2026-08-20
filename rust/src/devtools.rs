//! Deep debugging: performance traces with Web Vitals, network waterfalls with
//! bounded response bodies, computed styles, and HAR export.
//!
//! This is the gap against Chrome DevTools MCP. The existing `console_logs` /
//! `network_log` tools answer "what happened"; this module answers "why is it slow"
//! and "what exactly did that request return", which is what a developer debugging
//! their own app actually needs.
//!
//! Everything here is read-only and passes through [`mod@crate::trace::redact`] before it
//! leaves, because a network waterfall is made of headers and query strings — i.e. of
//! session tokens.
//!
//! Split by the question each part answers: [`timeline`] why the page is slow, [`network`]
//! what went over the wire, [`sourcemap`] where in the original source a minified location
//! is, and [`style`] which styles actually apply.

pub mod network;
pub mod sourcemap;
pub mod style;
pub mod timeline;

pub use network::{from_har, response_body, to_har};
pub use sourcemap::{decode_mappings, lookup_mapping, resolve_source, Mapping};
pub use style::computed_style;
pub use timeline::{cpu_profile, heap_stats, perf_trace};

#[cfg(test)]
mod tests {
    use super::sourcemap::vlq_decode;
    use super::timeline::derive_insights;
    use super::*;
    use serde_json::json;

    #[test]
    fn insights_flag_poor_web_vitals_against_published_thresholds() {
        let v = json!({
            "largest_contentful_paint": 5000.0,
            "cumulative_layout_shift": 0.4,
            "ttfb": 1200.0,
        });
        let out = derive_insights(&v);
        assert!(out.iter().any(|s| s.contains("LCP") && s.contains("poor")));
        assert!(out.iter().any(|s| s.contains("CLS") && s.contains("poor")));
        assert!(out.iter().any(|s| s.contains("TTFB")));
    }

    #[test]
    fn insights_distinguish_needs_improvement_from_poor() {
        let out = derive_insights(&json!({ "largest_contentful_paint": 3000.0 }));
        assert!(out.iter().any(|s| s.contains("needs improvement")));
        assert!(!out.iter().any(|s| s.contains("poor")));
    }

    /// A healthy page must say so explicitly. An empty list reads as "the tool did
    /// not work" rather than "nothing is wrong".
    #[test]
    fn a_healthy_page_gets_an_explicit_all_clear() {
        let out = derive_insights(&json!({
            "largest_contentful_paint": 1200.0,
            "cumulative_layout_shift": 0.01,
            "ttfb": 200.0,
        }));
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("no Web Vitals threshold was exceeded"));
    }

    #[test]
    fn missing_metrics_do_not_produce_bogus_insights() {
        // A page where the observer buffer had nothing: no metric, no verdict.
        let out = derive_insights(&json!({}));
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("no Web Vitals threshold"));
    }

    #[test]
    fn a_dominant_slow_resource_is_called_out() {
        let out = derive_insights(&json!({
            "slowest_resources": [{ "name": "https://cdn.test/huge.js", "ms": 3200.0 }],
        }));
        assert!(out.iter().any(|s| s.contains("huge.js")));
    }

    fn entry(url: &str) -> crate::capture::NetworkEntry {
        crate::capture::NetworkEntry {
            request_id: "1".into(),
            url: url.into(),
            method: "GET".into(),
            status: Some(200),
            status_text: "OK".into(),
            duration_ms: Some(50.0),
            encoded_data_length: Some(1024.0),
            timestamp: 0.0,
        }
    }

    #[test]
    fn har_export_is_well_formed() {
        let har = to_har(&[entry("https://example.com/a")], "https://example.com/");
        assert_eq!(har["log"]["version"], "1.2");
        assert_eq!(har["log"]["creator"]["name"], "NeoBrowser");
        assert_eq!(har["log"]["entries"][0]["request"]["method"], "GET");
        assert_eq!(har["log"]["entries"][0]["response"]["status"], 200);
        assert_eq!(har["log"]["pages"][0]["id"], "page_1");
    }

    /// A HAR is shared by construction, so a token in a URL must not ride along.
    #[test]
    fn har_export_redacts_urls() {
        let har = to_har(
            &[entry("https://api.test/cb?access_token=LEAKME&page=2")],
            "https://app.test/?session=ALSOLEAK",
        );
        let text = har.to_string();
        assert!(!text.contains("LEAKME"), "{text}");
        assert!(!text.contains("ALSOLEAK"), "{text}");
        // Non-secret parameters survive, or the export loses its diagnostic value.
        assert!(text.contains("page=2"));
    }

    #[test]
    fn a_non_har_document_is_rejected_clearly() {
        assert_eq!(from_har("not json")["ok"], false);
        assert_eq!(from_har(r#"{"hello":1}"#)["ok"], false);
        assert!(from_har(r#"{"hello":1}"#)["error"]
            .as_str()
            .unwrap()
            .contains("not a HAR"));
    }

    #[test]
    fn import_surfaces_failures_and_slow_requests() {
        let har = json!({ "log": { "entries": [
            { "time": 20.0, "request": { "url": "https://a.test/ok" },
              "response": { "status": 200, "bodySize": 100 } },
            { "time": 5000.0, "request": { "url": "https://a.test/slow" },
              "response": { "status": 200, "bodySize": 200 } },
            { "time": 10.0, "request": { "url": "https://a.test/gone" },
              "response": { "status": 404, "bodySize": 0 } },
            // An aborted request: status 0 must count as a failure, not be ignored.
            { "time": 1.0, "request": { "url": "https://a.test/blocked" },
              "response": { "status": 0, "bodySize": 0 } },
        ] } });
        let out = from_har(&har.to_string());
        assert_eq!(out["ok"], true);
        assert_eq!(out["entries"], 4);
        assert_eq!(out["total_body_bytes"], 300);
        assert_eq!(out["failures"].as_array().unwrap().len(), 2);
        assert_eq!(out["slowest"][0]["url"], "https://a.test/slow");
    }

    /// A HAR from someone else's session is full of tokens; importing must not echo
    /// them back into a transcript.
    #[test]
    fn imported_urls_are_redacted() {
        let har = json!({ "log": { "entries": [
            { "time": 1.0, "request": { "url": "https://a.test/?access_token=LEAKME" },
              "response": { "status": 500, "bodySize": 0 } },
        ] } });
        let text = from_har(&har.to_string()).to_string();
        assert!(!text.contains("LEAKME"), "{text}");
    }

    // --- source map VLQ ------------------------------------------------------

    /// The sign bit lives in the first character's LSB. Getting it wrong turns every
    /// negative delta into a huge positive one, so this is the load-bearing case.
    #[test]
    fn vlq_decodes_signed_values() {
        assert_eq!(vlq_decode(b"A").unwrap(), (0, 1));
        assert_eq!(vlq_decode(b"C").unwrap(), (1, 1));
        assert_eq!(vlq_decode(b"D").unwrap(), (-1, 1));
        assert_eq!(vlq_decode(b"E").unwrap(), (2, 1));
        assert_eq!(vlq_decode(b"F").unwrap(), (-2, 1));
        // Multi-character (continuation bit set on all but the last).
        assert_eq!(vlq_decode(b"2H").unwrap(), (123, 2));
        assert_eq!(vlq_decode(b"gB").unwrap(), (16, 2));
    }

    #[test]
    fn vlq_rejects_malformed_input() {
        assert_eq!(vlq_decode(b""), None);
        // Continuation bit set with nothing following.
        assert_eq!(vlq_decode(b"g"), None);
        // Not in the alphabet.
        assert_eq!(vlq_decode(b"!"), None);
        // A run of continuations must not shift past the accumulator.
        assert_eq!(vlq_decode(&[b'g'; 40]), None);
    }

    /// The field order is `[generatedColumn, sourceIndex, originalLine, originalColumn]`,
    /// all delta-encoded. Mixing up the middle two is the classic bug: it reports the
    /// right line number against the wrong file.
    #[test]
    fn mappings_decode_with_correct_field_order() {
        // `ACAA` = [genCol +0, sourceIndex +1, origLine +0, origCol +0].
        let m = decode_mappings("AAAA;ACAA");
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].generated_line, 0);
        assert_eq!(m[0].source_index, 0);
        assert_eq!(m[0].original_line, 0);
        assert_eq!(m[1].generated_line, 1);
        assert_eq!(m[1].source_index, 1, "the second field is the SOURCE index");
        assert_eq!(
            m[1].original_line, 0,
            "the original line did not advance here"
        );
    }

    /// The original-line counter carries across generated lines. If it were reset per
    /// line, every mapping after the first would point at the top of its file.
    #[test]
    fn the_original_line_counter_carries_across_generated_lines() {
        // `AACA` = [genCol +0, sourceIndex +0, origLine +1, origCol +0], twice.
        let m = decode_mappings("AACA;AACA;AACA");
        assert_eq!(m.len(), 3);
        assert_eq!(m[0].original_line, 1);
        assert_eq!(m[1].original_line, 2, "must accumulate, not reset");
        assert_eq!(m[2].original_line, 3);
    }

    #[test]
    fn the_generated_column_resets_per_line_but_others_do_not() {
        // Line 0: columns 0 then 2 (delta 1 -> encoded 'C' is +1 after the >>1).
        let m = decode_mappings("AAAA,CAAA;AAAA");
        assert_eq!(m[0].generated_column, 0);
        assert_eq!(
            m[1].generated_column, 1,
            "second segment advances the column"
        );
        assert_eq!(
            m[2].generated_column, 0,
            "a new generated line must restart the column"
        );
    }

    #[test]
    fn segments_without_an_original_position_are_skipped() {
        // A single-field segment marks generated-only code.
        let m = decode_mappings("A;AAAA");
        assert_eq!(m.len(), 1, "the one-field segment has no original position");
        assert_eq!(m[0].generated_line, 1);
    }

    #[test]
    fn empty_and_corrupt_mappings_do_not_panic() {
        assert!(decode_mappings("").is_empty());
        assert!(decode_mappings(";;;").is_empty());
        assert!(decode_mappings("!!!,???").is_empty());
        // Negative deltas walking off the start are dropped, not reported as garbage.
        assert!(decode_mappings("AAAD").is_empty());
    }

    /// Mappings mark region STARTS, so a lookup must find the last one at or before the
    /// column. An exact-match lookup would fail for almost every real position.
    #[test]
    fn lookup_finds_the_covering_mapping_not_an_exact_match() {
        let mappings = vec![
            Mapping {
                generated_line: 0,
                generated_column: 0,
                source_index: 0,
                original_line: 10,
                original_column: 0,
            },
            Mapping {
                generated_line: 0,
                generated_column: 50,
                source_index: 0,
                original_line: 20,
                original_column: 0,
            },
            Mapping {
                generated_line: 1,
                generated_column: 0,
                source_index: 0,
                original_line: 30,
                original_column: 0,
            },
        ];
        // Column 60 is inside the region that starts at 50.
        assert_eq!(lookup_mapping(&mappings, 0, 60).unwrap().original_line, 20);
        // Column 10 is inside the region that starts at 0.
        assert_eq!(lookup_mapping(&mappings, 0, 10).unwrap().original_line, 10);
        // A different line uses that line's mappings only.
        assert_eq!(lookup_mapping(&mappings, 1, 99).unwrap().original_line, 30);
        // A line with no mappings yields nothing rather than the nearest guess.
        assert!(lookup_mapping(&mappings, 5, 0).is_none());
    }
}
