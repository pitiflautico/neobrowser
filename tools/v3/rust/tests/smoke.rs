//! Smoke tests for neobrowser CLI commands.
//! Run: cargo test

use std::process::Command;

fn neo() -> Command {
    Command::new(env!("CARGO_BIN_EXE_neobrowser_rs"))
}

#[test]
fn help_works() {
    let out = neo().arg("--help").output().unwrap();
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("neobrowser"));
    assert!(s.contains("setup"));
    assert!(s.contains("mcp"));
}

#[test]
fn fetch_example_com() {
    let out = neo()
        .args(["fetch", "https://example.com"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("Example Domain"));
}

#[test]
fn wom_runs_without_crash() {
    let out = neo()
        .args(["wom", "https://example.com", "--compact"])
        .output()
        .unwrap();
    // WOM light mode may produce empty output for simple pages — just check no crash
    assert!(out.status.success());
}

#[test]
fn see_example_com() {
    let out = neo()
        .args(["see", "https://example.com"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("Example Domain"));
}

#[test]
fn mcp_initialize() {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = neo()
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#;

    {
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(stdin, "{init}").unwrap();
    }

    // Read response
    let output = child.wait_with_output().unwrap();
    let s = String::from_utf8_lossy(&output.stdout);
    assert!(s.contains("neobrowser"), "MCP init failed: {s}");
}

#[test]
fn setup_help() {
    let out = neo().args(["setup", "--help"]).output().unwrap();
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("setup"));
    assert!(s.contains("sites"));
}
