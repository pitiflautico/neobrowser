//! Pin the `unsafe` inventory.
//!
//! An auditor's first question about a Rust codebase is "where is the unsafe, and why is
//! each block sound". This test makes both answerable mechanically, and makes adding a
//! new `unsafe` block a deliberate act rather than something that slips through review.
//!
//! Every block must carry a `SAFETY:` comment. That is not bureaucracy: the two genuinely
//! dangerous calls here are `libc::kill` (where a non-positive pid signals a whole process
//! group instead of one child) and the Windows DPAPI path (raw pointer into a
//! caller-allocated buffer). Both are sound for reasons that are not visible from the
//! call site, which is exactly when a comment is load-bearing.

use std::path::Path;

/// Every `.rs` file under `src/`, recursively.
fn sources(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(sources(&p));
        } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
            out.push(p);
        }
    }
    out
}

/// Locations of `unsafe` blocks, with whether a SAFETY comment precedes each.
fn unsafe_sites() -> Vec<(String, usize, bool)> {
    let mut out = Vec::new();
    for path in sources(Path::new("src")) {
        let text = std::fs::read_to_string(&path).expect("readable source");
        let name = path
            .strip_prefix("src")
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            // `unsafe` as an expression or block, not the word inside a comment or string.
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || !line.contains("unsafe ") && !line.contains("unsafe{") {
                continue;
            }
            // A SAFETY note may sit a few lines above, past a `#[cfg]` attribute.
            let justified = lines[i.saturating_sub(6)..i]
                .iter()
                .any(|l| l.contains("SAFETY:"));
            out.push((name.clone(), i + 1, justified));
        }
    }
    out
}

/// Every `unsafe` block carries a SAFETY justification.
#[test]
fn every_unsafe_block_is_justified() {
    let sites = unsafe_sites();
    assert!(
        !sites.is_empty(),
        "found no unsafe blocks at all, which means this detector is broken — and a broken \
         detector passes silently while checking nothing"
    );
    let unjustified: Vec<String> = sites
        .iter()
        .filter(|(_, _, justified)| !justified)
        .map(|(f, l, _)| format!("{f}:{l}"))
        .collect();
    assert!(
        unjustified.is_empty(),
        "unsafe without a `SAFETY:` comment explaining why it is sound:\n  {}",
        unjustified.join("\n  ")
    );
}

/// The inventory stays small. This is a ratchet, not a limit: if a change genuinely needs
/// more unsafe, raise the number in the same commit and say why in the message — the point
/// is that it cannot grow without anyone noticing.
#[test]
fn the_unsafe_inventory_does_not_grow_unnoticed() {
    const EXPECTED: usize = 8;
    let sites = unsafe_sites();
    assert!(
        sites.len() <= EXPECTED,
        "unsafe blocks went from {EXPECTED} to {}. All of them are FFI (libc signal and \
         euid probes on unix, DPAPI on Windows); there is no unsafe in the CDP, policy, \
         vault or action paths, and keeping it that way is deliberate. New sites:\n  {}",
        sites.len(),
        sites
            .iter()
            .map(|(f, l, _)| format!("{f}:{l}"))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}
